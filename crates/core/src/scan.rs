//! The scanner: compares what is on disk with the index and records what
//! changed, hashing only files whose size or mtime moved.
//!
//! It runs in phases so the engine lock is held only for the comparisons
//! and never across disk IO: `walk_paths` (disk), `apply_walk` (index),
//! `hash_files` (disk), `apply_hashes` (index). `scan_paths` runs all four
//! back to back for callers that hold nothing.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tracing::warn;
use walkdir::WalkDir;

use crate::clock::mtime_ms;
use crate::hash::blake3_file;
use crate::ignore::is_ignored;
use crate::index::{Entry, EntryKind, Index};
use crate::paths::{is_under, join_rel, normalize_rel, parent_of};
use crate::vv::bump;

#[derive(Debug, Default)]
pub struct ScanOutcome {
    /// Entries whose version vector was bumped, in index form.
    pub changed: Vec<Entry>,
    /// Relative directories whose listing changed (`""` is the root).
    pub dirs_touched: BTreeSet<String>,
}

impl ScanOutcome {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
    }

    pub fn absorb(&mut self, other: ScanOutcome) {
        self.changed.extend(other.changed);
        self.dirs_touched.extend(other.dirs_touched);
    }
}

/// One item found on disk.
#[derive(Debug, Clone)]
pub struct DiskItem {
    pub rel: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootKind {
    Dir,
    File,
    /// Missing, a symbolic link or a special file: not synced.
    Gone,
}

/// Everything a walk found, before the index is consulted.
#[derive(Debug, Default)]
pub struct Walked {
    items: Vec<DiskItem>,
    roots: Vec<(String, RootKind)>,
    /// Directories that could not be read. Nothing under them is
    /// tombstoned, since their contents are unknown rather than gone.
    pub unreadable: Vec<String>,
}

/// A file that must be hashed before the index can be updated, with the
/// index entry the decision was based on.
#[derive(Debug, Clone)]
pub struct ToHash {
    item: DiskItem,
    expected: Option<Entry>,
}

#[derive(Debug)]
pub struct Hashed {
    item: DiskItem,
    expected: Option<Entry>,
    hash: String,
}

/// Scans the whole folder and tombstones every index entry no longer on disk.
pub async fn scan_all(
    root: &Path,
    index: &mut Index,
    device_id: &str,
    now_ms: i64,
) -> Result<ScanOutcome> {
    scan_paths(root, index, device_id, &[String::new()], now_ms).await
}

/// Rescans the given relative paths. A directory is walked; a file is
/// compared; a missing path is tombstoned together with everything under it.
pub async fn scan_paths(
    root: &Path,
    index: &mut Index,
    device_id: &str,
    rels: &[String],
    now_ms: i64,
) -> Result<ScanOutcome> {
    let walked = walk_paths(root, rels).await?;
    let (mut out, to_hash) = apply_walk(index, &walked, device_id, now_ms);
    let hashed = hash_files(root, to_hash).await;
    apply_hashes(index, hashed, device_id, now_ms, &mut out);
    Ok(out)
}

/// Phase one, disk only: lists what is under each path, plus the
/// ancestors of each path so `a/b/c` indexes `a` and `a/b` as well.
pub async fn walk_paths(root: &Path, rels: &[String]) -> Result<Walked> {
    let rels = dedup(rels);
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || walk_all(&root, &rels))
        .await
        .context("walk task failed")?
}

fn walk_all(root: &Path, rels: &[String]) -> Result<Walked> {
    let mut walked = Walked::default();
    let mut ancestors_seen: HashSet<String> = HashSet::new();
    for rel in rels {
        if is_ignored(rel) {
            continue;
        }
        let mut prefix = String::new();
        for component in parent_of(rel).split('/').filter(|c| !c.is_empty()) {
            prefix = join_rel(&prefix, component);
            if !ancestors_seen.insert(prefix.clone()) {
                continue;
            }
            if let Ok(md) = std::fs::symlink_metadata(root.join(&prefix)) {
                if md.is_dir() {
                    walked.items.push(DiskItem {
                        rel: prefix.clone(),
                        is_dir: true,
                        size: 0,
                        mtime_ms: mtime_ms(&md),
                    });
                }
            }
        }

        let abs = abs_path(root, rel);
        match std::fs::symlink_metadata(&abs) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                walked.roots.push((rel.clone(), RootKind::Gone))
            }
            Err(e) => return Err(e).with_context(|| format!("stat {}", abs.display())),
            Ok(md) if md.is_dir() => {
                walk_dir(root, rel, &mut walked)?;
                walked.roots.push((rel.clone(), RootKind::Dir));
            }
            Ok(md) if md.is_file() => {
                walked.items.push(DiskItem {
                    rel: rel.clone(),
                    is_dir: false,
                    size: md.len(),
                    mtime_ms: mtime_ms(&md),
                });
                walked.roots.push((rel.clone(), RootKind::File));
            }
            Ok(_) => walked.roots.push((rel.clone(), RootKind::Gone)),
        }
    }
    Ok(walked)
}

/// Drops any path that lies under another path in the list, since walking
/// the parent covers it.
fn dedup(rels: &[String]) -> Vec<String> {
    let mut sorted: Vec<String> = rels.to_vec();
    sorted.sort();
    sorted.dedup();
    let mut kept: Vec<String> = Vec::new();
    for rel in sorted {
        if kept.iter().any(|k| is_under(&rel, k)) {
            continue;
        }
        kept.push(rel);
    }
    kept
}

fn abs_path(root: &Path, rel: &str) -> PathBuf {
    if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    }
}

/// Lists everything under `rel`, including `rel` itself when it is not the
/// root, skipping ignored names and symbolic links. A directory that cannot
/// be read is recorded as unreadable and skipped; the walked root itself
/// being unreadable fails the scan.
fn walk_dir(root: &Path, rel: &str, walked: &mut Walked) -> Result<()> {
    let start = abs_path(root, rel);
    let walker = WalkDir::new(&start)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| match normalize_rel(e.path(), root) {
            Some(r) => !is_ignored(&r),
            None => false,
        });
    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                let failed = e.path().and_then(|p| normalize_rel(p, root));
                match failed {
                    Some(r) if r == rel => bail!("cannot read {}: {e}", describe(rel)),
                    Some(r) => {
                        warn!("scan: cannot read {r}; leaving its contents as they are: {e}");
                        walked.unreadable.push(r);
                    }
                    None => bail!("cannot read {}: {e}", describe(rel)),
                }
                continue;
            }
        };
        let Some(item_rel) = normalize_rel(entry.path(), root) else {
            continue;
        };
        if item_rel.is_empty() {
            continue;
        }
        let ft = entry.file_type();
        if ft.is_symlink() || !(ft.is_dir() || ft.is_file()) {
            continue;
        }
        let md = match entry.metadata() {
            Ok(md) => md,
            Err(e) => {
                warn!("scan: cannot stat {}: {e}", entry.path().display());
                continue;
            }
        };
        walked.items.push(DiskItem {
            rel: item_rel,
            is_dir: ft.is_dir(),
            size: if ft.is_dir() { 0 } else { md.len() },
            mtime_ms: mtime_ms(&md),
        });
    }
    Ok(())
}

fn describe(rel: &str) -> &str {
    if rel.is_empty() {
        "the folder"
    } else {
        rel
    }
}

/// Phase two, index only: records new directories and tombstones for
/// entries that are gone, and lists the files that need a hash.
pub fn apply_walk(
    index: &mut Index,
    walked: &Walked,
    device_id: &str,
    now_ms: i64,
) -> (ScanOutcome, Vec<ToHash>) {
    let mut out = ScanOutcome::default();
    let mut to_hash = Vec::new();
    let mut seen: HashSet<&str> = walked.items.iter().map(|i| i.rel.as_str()).collect();
    for (rel, kind) in &walked.roots {
        if *kind != RootKind::Gone {
            seen.insert(rel.as_str());
        }
    }

    for item in &walked.items {
        let existing = index.get(&item.rel);
        if item.is_dir {
            if existing.is_some_and(|e| e.is_live_dir()) {
                continue;
            }
            let mut vv = existing.map(|e| e.vv.clone()).unwrap_or_default();
            bump(&mut vv, device_id, index.floor());
            record_change(
                index,
                Entry {
                    path: item.rel.clone(),
                    kind: EntryKind::Dir,
                    size: 0,
                    mtime_ms: item.mtime_ms,
                    hash: None,
                    deleted: false,
                    vv,
                    seen_at_ms: now_ms,
                },
                &mut out,
            );
        } else {
            if existing.is_some_and(|e| {
                e.is_live_file() && e.size == item.size && e.mtime_ms == item.mtime_ms
            }) {
                continue;
            }
            to_hash.push(ToHash {
                item: item.clone(),
                expected: existing.cloned(),
            });
        }
    }

    for (rel, _) in &walked.roots {
        let missing: Vec<String> = index
            .live_within(rel)
            .into_iter()
            .filter(|e| !seen.contains(e.path.as_str()))
            .filter(|e| {
                !walked
                    .unreadable
                    .iter()
                    .any(|u| e.path != *u && is_under(&e.path, u))
            })
            .map(|e| e.path.clone())
            .collect();
        for path in missing {
            tombstone(index, &path, device_id, now_ms, &mut out);
        }
    }
    (out, to_hash)
}

/// Phase three, disk only. A file that changes while it is being hashed
/// is dropped: the hash belongs to no state the index could describe, and
/// the watcher reports the change again.
pub async fn hash_files(root: &Path, to_hash: Vec<ToHash>) -> Vec<Hashed> {
    let mut out = Vec::with_capacity(to_hash.len());
    for th in to_hash {
        let abs = root.join(&th.item.rel);
        let hash = match blake3_file(&abs).await {
            Ok(h) => h,
            Err(e) => {
                warn!("scan: cannot hash {}: {e}", th.item.rel);
                continue;
            }
        };
        match tokio::fs::symlink_metadata(&abs).await {
            Ok(md)
                if md.is_file()
                    && md.len() == th.item.size
                    && mtime_ms(&md) == th.item.mtime_ms =>
            {
                out.push(Hashed {
                    item: th.item,
                    expected: th.expected,
                    hash,
                });
            }
            _ => warn!("scan: {} changed while it was hashed", th.item.rel),
        }
    }
    out
}

/// Phase four, index only. A path whose index entry moved on since the
/// plan (a peer's version was installed meanwhile) is left alone.
pub fn apply_hashes(
    index: &mut Index,
    hashed: Vec<Hashed>,
    device_id: &str,
    now_ms: i64,
    out: &mut ScanOutcome,
) {
    for h in hashed {
        if index.get(&h.item.rel) != h.expected.as_ref() {
            continue;
        }
        if let Some(e) = &h.expected {
            if e.is_live_file() && e.hash.as_deref() == Some(h.hash.as_str()) {
                // Touched but not changed: keep the version vector so peers
                // do not fetch it again, just remember the new metadata.
                let mut updated = e.clone();
                updated.size = h.item.size;
                updated.mtime_ms = h.item.mtime_ms;
                updated.seen_at_ms = now_ms;
                index.insert(updated);
                continue;
            }
        }
        let mut vv = h.expected.map(|e| e.vv).unwrap_or_default();
        bump(&mut vv, device_id, index.floor());
        record_change(
            index,
            Entry {
                path: h.item.rel.clone(),
                kind: EntryKind::File,
                size: h.item.size,
                mtime_ms: h.item.mtime_ms,
                hash: Some(h.hash),
                deleted: false,
                vv,
                seen_at_ms: now_ms,
            },
            out,
        );
    }
}

pub fn tombstone(
    index: &mut Index,
    path: &str,
    device_id: &str,
    now_ms: i64,
    out: &mut ScanOutcome,
) {
    let Some(e) = index.get(path) else {
        return;
    };
    if e.deleted {
        return;
    }
    let mut t = e.clone();
    t.deleted = true;
    t.size = 0;
    t.hash = None;
    t.mtime_ms = now_ms;
    t.seen_at_ms = now_ms;
    bump(&mut t.vv, device_id, index.floor());
    if t.kind == EntryKind::Dir {
        out.dirs_touched.insert(t.path.clone());
    }
    record_change(index, t, out);
}

fn record_change(index: &mut Index, entry: Entry, out: &mut ScanOutcome) {
    out.dirs_touched.insert(parent_of(&entry.path).to_string());
    index.insert(entry.clone());
    out.changed.push(entry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::blake3_hex;
    use filetime::FileTime;
    use std::collections::BTreeMap;

    const DEV: &str = "dev-a";

    #[tokio::test]
    async fn new_file_is_hashed_and_bumped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        let mut index = Index::default();
        let out = scan_all(dir.path(), &mut index, DEV, 1).await.unwrap();
        assert_eq!(out.changed.len(), 1);
        assert_eq!(out.dirs_touched, BTreeSet::from([String::new()]));
        let e = index.get("a.txt").unwrap();
        assert_eq!(e.kind, EntryKind::File);
        assert_eq!(e.size, 5);
        assert_eq!(e.hash.as_deref(), Some(blake3_hex(b"hello").as_str()));
        assert_eq!(e.vv, BTreeMap::from([(DEV.to_string(), 1)]));
        assert!(!e.deleted);
        assert_eq!(e.seen_at_ms, 1);
    }

    #[tokio::test]
    async fn unchanged_file_is_not_rehashed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        let mut index = Index::default();
        scan_all(dir.path(), &mut index, DEV, 1).await.unwrap();
        let before = index.get("a.txt").cloned().unwrap();
        let walked = walk_paths(dir.path(), &[String::new()]).await.unwrap();
        let (out, to_hash) = apply_walk(&mut index, &walked, DEV, 2);
        assert!(out.changed.is_empty());
        assert!(to_hash.is_empty(), "nothing to hash");
        assert_eq!(index.get("a.txt"), Some(&before));
    }

    #[tokio::test]
    async fn touch_without_content_change_keeps_vv() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();
        let mut index = Index::default();
        scan_all(dir.path(), &mut index, DEV, 1).await.unwrap();
        let before = index.get("a.txt").cloned().unwrap();

        let later = FileTime::from_unix_time(before.mtime_ms / 1000 + 100, 0);
        filetime::set_file_mtime(&path, later).unwrap();
        let out = scan_all(dir.path(), &mut index, DEV, 2).await.unwrap();
        assert!(out.changed.is_empty());
        let after = index.get("a.txt").unwrap();
        assert_eq!(after.vv, before.vv);
        assert_eq!(after.mtime_ms, (before.mtime_ms / 1000 + 100) * 1000);
        assert_eq!(after.hash, before.hash);
    }

    #[tokio::test]
    async fn modified_file_is_bumped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();
        let mut index = Index::default();
        scan_all(dir.path(), &mut index, DEV, 1).await.unwrap();
        std::fs::write(&path, b"hello world").unwrap();
        let out = scan_all(dir.path(), &mut index, DEV, 2).await.unwrap();
        assert_eq!(out.changed.len(), 1);
        let e = index.get("a.txt").unwrap();
        assert_eq!(e.size, 11);
        assert_eq!(e.vv, BTreeMap::from([(DEV.to_string(), 2)]));
    }

    #[tokio::test]
    async fn deleted_file_becomes_tombstone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();
        let mut index = Index::default();
        scan_all(dir.path(), &mut index, DEV, 1).await.unwrap();
        std::fs::remove_file(&path).unwrap();
        let out = scan_all(dir.path(), &mut index, DEV, 2).await.unwrap();
        assert_eq!(out.changed.len(), 1);
        let e = index.get("a.txt").unwrap();
        assert!(e.deleted);
        assert_eq!(e.hash, None);
        assert_eq!(e.vv, BTreeMap::from([(DEV.to_string(), 2)]));
        assert_eq!(e.seen_at_ms, 2);
    }

    #[tokio::test]
    async fn removed_directory_tombstones_its_children_via_scan_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("d/e")).unwrap();
        std::fs::write(dir.path().join("d/e/x.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("keep.txt"), b"k").unwrap();
        let mut index = Index::default();
        scan_all(dir.path(), &mut index, DEV, 1).await.unwrap();
        assert_eq!(index.summary(), (2, 2, 2));

        std::fs::remove_dir_all(dir.path().join("d")).unwrap();
        let out = scan_paths(dir.path(), &mut index, DEV, &["d".to_string()], 2)
            .await
            .unwrap();
        assert_eq!(out.changed.len(), 3);
        assert!(out.changed.iter().all(|e| e.deleted));
        assert!(index.get("d/e/x.txt").unwrap().deleted);
        assert!(!index.get("keep.txt").unwrap().deleted);
        assert!(out.dirs_touched.contains(""));
        assert!(out.dirs_touched.contains("d"));
    }

    #[tokio::test]
    async fn scanning_a_deep_path_indexes_its_ancestors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("photos/2026")).unwrap();
        std::fs::write(dir.path().join("photos/other.txt"), b"o").unwrap();
        let mut index = Index::default();
        let out = scan_paths(dir.path(), &mut index, DEV, &["photos/2026".to_string()], 1)
            .await
            .unwrap();
        let mut changed: Vec<&str> = out.changed.iter().map(|e| e.path.as_str()).collect();
        changed.sort();
        assert_eq!(changed, vec!["photos", "photos/2026"]);
        assert!(index.get("photos").unwrap().is_live_dir());
        // The sibling was not walked, so it is neither indexed nor touched.
        assert!(index.get("photos/other.txt").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unreadable_subdirectory_keeps_its_children() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("locked")).unwrap();
        std::fs::write(dir.path().join("locked/inside.txt"), b"i").unwrap();
        std::fs::write(dir.path().join("open.txt"), b"o").unwrap();
        let mut index = Index::default();
        scan_all(dir.path(), &mut index, DEV, 1).await.unwrap();
        assert_eq!(index.summary(), (2, 1, 2));

        std::fs::set_permissions(dir.path().join("locked"), PermissionsExt::from_mode(0o000))
            .unwrap();
        if std::fs::read_dir(dir.path().join("locked")).is_ok() {
            // Running as root: permissions do not bite, nothing to test.
            std::fs::set_permissions(dir.path().join("locked"), PermissionsExt::from_mode(0o755))
                .unwrap();
            return;
        }
        let walked = walk_paths(dir.path(), &[String::new()]).await.unwrap();
        assert_eq!(walked.unreadable, vec!["locked".to_string()]);
        let (out, _) = apply_walk(&mut index, &walked, DEV, 2);
        assert!(out.changed.is_empty(), "{:?}", out.changed);
        assert!(!index.get("locked/inside.txt").unwrap().deleted);
        assert!(!index.get("locked").unwrap().deleted);

        std::fs::set_permissions(dir.path().join("locked"), PermissionsExt::from_mode(0o755))
            .unwrap();
        std::fs::remove_dir_all(dir.path().join("locked")).unwrap();
        let out = scan_all(dir.path(), &mut index, DEV, 3).await.unwrap();
        assert_eq!(out.changed.len(), 2);
        assert!(index.get("locked/inside.txt").unwrap().deleted);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unreadable_root_fails_the_scan_instead_of_tombstoning() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        let mut index = Index::default();
        scan_all(dir.path(), &mut index, DEV, 1).await.unwrap();
        std::fs::set_permissions(dir.path(), PermissionsExt::from_mode(0o000)).unwrap();
        let readable = std::fs::read_dir(dir.path()).is_ok();
        let result = scan_all(dir.path(), &mut index, DEV, 2).await;
        std::fs::set_permissions(dir.path(), PermissionsExt::from_mode(0o755)).unwrap();
        if readable {
            return;
        }
        assert!(result.is_err());
        assert!(!index.get("a.txt").unwrap().deleted);
    }

    #[tokio::test]
    async fn a_file_that_changes_while_hashing_is_left_for_the_next_scan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();
        let mut index = Index::default();
        let walked = walk_paths(dir.path(), &[String::new()]).await.unwrap();
        let (mut out, to_hash) = apply_walk(&mut index, &walked, DEV, 1);
        assert_eq!(to_hash.len(), 1);
        // Changes size between the walk and the hash.
        std::fs::write(&path, b"hello world").unwrap();
        let hashed = hash_files(dir.path(), to_hash).await;
        assert!(hashed.is_empty());
        apply_hashes(&mut index, hashed, DEV, 1, &mut out);
        assert!(index.get("a.txt").is_none());
    }

    #[tokio::test]
    async fn an_index_entry_that_moved_on_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        let mut index = Index::default();
        let walked = walk_paths(dir.path(), &[String::new()]).await.unwrap();
        let (mut out, to_hash) = apply_walk(&mut index, &walked, DEV, 1);
        let hashed = hash_files(dir.path(), to_hash).await;
        // A peer's version was installed for the same path meanwhile.
        let installed = Entry {
            path: "a.txt".into(),
            kind: EntryKind::File,
            size: 5,
            mtime_ms: 42,
            hash: Some("peer".into()),
            deleted: false,
            vv: BTreeMap::from([("peer".to_string(), 3)]),
            seen_at_ms: 1,
        };
        index.insert(installed.clone());
        apply_hashes(&mut index, hashed, DEV, 1, &mut out);
        assert_eq!(index.get("a.txt"), Some(&installed));
        assert!(out.changed.is_empty());
    }

    #[tokio::test]
    async fn every_bump_lands_above_the_floor() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        std::fs::create_dir_all(dir.path().join("d")).unwrap();
        let mut index = Index::default();
        index.raise_floor(500);
        scan_all(dir.path(), &mut index, DEV, 1).await.unwrap();
        assert_eq!(index.get("a.txt").unwrap().vv[DEV], 501);
        assert_eq!(index.get("d").unwrap().vv[DEV], 501);
        std::fs::remove_file(dir.path().join("a.txt")).unwrap();
        scan_all(dir.path(), &mut index, DEV, 2).await.unwrap();
        assert_eq!(index.get("a.txt").unwrap().vv[DEV], 502);
    }

    #[tokio::test]
    async fn ignored_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".owl-tmp-x"), b"partial").unwrap();
        std::fs::create_dir_all(dir.path().join(".owl")).unwrap();
        std::fs::write(dir.path().join(".owl/meta"), b"m").unwrap();
        std::fs::write(dir.path().join("Thumbs.db"), b"t").unwrap();
        std::fs::write(dir.path().join("real.txt"), b"r").unwrap();
        let mut index = Index::default();
        scan_all(dir.path(), &mut index, DEV, 1).await.unwrap();
        let paths: Vec<&str> = index.entries().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["real.txt"]);
    }

    #[tokio::test]
    async fn dedup_drops_covered_paths() {
        let rels = vec![
            "a/b".to_string(),
            "a".to_string(),
            "c".to_string(),
            "a/b/c".to_string(),
        ];
        assert_eq!(dedup(&rels), vec!["a".to_string(), "c".to_string()]);
        assert_eq!(
            dedup(&["x".to_string(), String::new()]),
            vec![String::new()]
        );
    }
}
