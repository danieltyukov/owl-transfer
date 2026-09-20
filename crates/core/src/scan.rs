//! The scanner: compares what is on disk with the index and records what
//! changed, hashing only files whose size or mtime moved.

use std::collections::{BTreeSet, HashSet};
use std::fs::Metadata;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::warn;
use walkdir::WalkDir;

use crate::clock::system_time_ms;
use crate::hash::blake3_file;
use crate::ignore::is_ignored;
use crate::index::{Entry, EntryKind, Index};
use crate::paths::{is_under, normalize_rel, parent_of};
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
}

/// One item found on disk.
struct DiskItem {
    rel: String,
    is_dir: bool,
    size: u64,
    mtime_ms: i64,
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
    let mut out = ScanOutcome::default();
    for rel in dedup(rels) {
        if is_ignored(&rel) {
            continue;
        }
        let abs = abs_path(root, &rel);
        let md = match tokio::fs::symlink_metadata(&abs).await {
            Ok(md) => Some(md),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("stat {}", abs.display())),
        };
        match md {
            Some(md) if md.is_dir() => {
                scan_dir(root, &rel, index, device_id, now_ms, &mut out).await?
            }
            Some(md) if md.is_file() => {
                let item = DiskItem {
                    rel: rel.clone(),
                    is_dir: false,
                    size: md.len(),
                    mtime_ms: mtime_of(&md),
                };
                compare_item(root, &item, index, device_id, now_ms, &mut out).await;
                // Anything the index still holds under this path belonged to
                // a directory that is now a file.
                let seen: HashSet<String> = HashSet::from([rel.clone()]);
                tombstone_missing(index, &rel, &seen, device_id, now_ms, &mut out);
            }
            // Symbolic links and special files are not synced.
            _ => tombstone_missing(index, &rel, &HashSet::new(), device_id, now_ms, &mut out),
        }
    }
    Ok(out)
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

fn mtime_of(md: &Metadata) -> i64 {
    md.modified().map(system_time_ms).unwrap_or(0)
}

async fn scan_dir(
    root: &Path,
    rel: &str,
    index: &mut Index,
    device_id: &str,
    now_ms: i64,
    out: &mut ScanOutcome,
) -> Result<()> {
    let items = {
        let root = root.to_path_buf();
        let rel = rel.to_string();
        tokio::task::spawn_blocking(move || walk(&root, &rel))
            .await
            .context("walk task failed")?
    };
    let mut seen: HashSet<String> = HashSet::with_capacity(items.len() + 1);
    seen.insert(rel.to_string());
    for item in &items {
        seen.insert(item.rel.clone());
        compare_item(root, item, index, device_id, now_ms, out).await;
    }
    tombstone_missing(index, rel, &seen, device_id, now_ms, out);
    Ok(())
}

/// Lists everything under `rel`, including `rel` itself when it is not the
/// root, skipping ignored names and symbolic links. Unreadable entries are
/// logged and skipped rather than failing the whole scan.
fn walk(root: &Path, rel: &str) -> Vec<DiskItem> {
    let start = abs_path(root, rel);
    let mut items = Vec::new();
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
                warn!("scan: skipping unreadable entry: {e}");
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
        items.push(DiskItem {
            rel: item_rel,
            is_dir: ft.is_dir(),
            size: if ft.is_dir() { 0 } else { md.len() },
            mtime_ms: mtime_of(&md),
        });
    }
    items
}

async fn compare_item(
    root: &Path,
    item: &DiskItem,
    index: &mut Index,
    device_id: &str,
    now_ms: i64,
    out: &mut ScanOutcome,
) {
    let existing = index.get(&item.rel).cloned();
    if item.is_dir {
        if matches!(&existing, Some(e) if e.is_live_dir()) {
            return;
        }
        let mut vv = existing.map(|e| e.vv).unwrap_or_default();
        bump(&mut vv, device_id);
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
            out,
        );
        return;
    }

    if let Some(e) = &existing {
        if e.is_live_file() && e.size == item.size && e.mtime_ms == item.mtime_ms {
            return;
        }
    }
    let hash = match blake3_file(&abs_path(root, &item.rel)).await {
        Ok(h) => h,
        Err(e) => {
            // The file may have vanished between the walk and the hash; the
            // watcher will report its removal separately.
            warn!("scan: cannot hash {}: {e}", item.rel);
            return;
        }
    };
    if let Some(e) = &existing {
        if e.is_live_file() && e.hash.as_deref() == Some(hash.as_str()) {
            // Touched but not changed: keep the version vector so peers do
            // not fetch it again, just remember the new metadata.
            let mut updated = e.clone();
            updated.size = item.size;
            updated.mtime_ms = item.mtime_ms;
            updated.seen_at_ms = now_ms;
            index.insert(updated);
            return;
        }
    }
    let mut vv = existing.map(|e| e.vv).unwrap_or_default();
    bump(&mut vv, device_id);
    record_change(
        index,
        Entry {
            path: item.rel.clone(),
            kind: EntryKind::File,
            size: item.size,
            mtime_ms: item.mtime_ms,
            hash: Some(hash),
            deleted: false,
            vv,
            seen_at_ms: now_ms,
        },
        out,
    );
}

/// Tombstones every live index entry that is `dir` or under it and is not in
/// `seen`.
fn tombstone_missing(
    index: &mut Index,
    dir: &str,
    seen: &HashSet<String>,
    device_id: &str,
    now_ms: i64,
    out: &mut ScanOutcome,
) {
    let missing: Vec<String> = index
        .live_within(dir)
        .into_iter()
        .filter(|e| !seen.contains(&e.path))
        .map(|e| e.path.clone())
        .collect();
    for path in missing {
        tombstone(index, &path, device_id, now_ms, out);
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
    bump(&mut t.vv, device_id);
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
        let out = scan_all(dir.path(), &mut index, DEV, 2).await.unwrap();
        assert!(out.changed.is_empty());
        assert!(out.dirs_touched.is_empty());
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
