//! Local changes: scanning what the watcher reports, and the file
//! operations the interface performs, which are indexed at once rather
//! than waiting for the watcher.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncRead, AsyncWriteExt};
use tracing::debug;
use unicode_normalization::UnicodeNormalization;

use super::inner::{Inner, Settings, RECENT_MS};
use super::Engine;
use crate::clock::{mtime_ms, now_ms};
use crate::ignore::is_ignored;
use crate::index::{Entry, Index};
use crate::paths::{
    file_name_of, is_conflict_name, join_rel, parent_of, validate_name, validate_rel,
};
use crate::proto::{chunk_entries, Control, Frame};
use crate::scan::{apply_hashes, apply_walk, hash_files, walk_paths, ScanOutcome};
use crate::state::{DirEntry, EntryStatus};
use crate::vv::VersionVector;
use crate::watch::Watcher;

impl Engine {
    /// Starts watching the folder, then indexes it in full. The watcher
    /// goes first so nothing that changes during the scan is missed.
    pub(crate) async fn start_folder(&self) -> Result<()> {
        let settings = self.settings();
        let watcher = Watcher::start(
            settings.folder.clone(),
            self.shared.poll_watch,
            self.shared.watch_tx.clone(),
        )?;
        self.lock().await.watcher = Some(watcher);
        self.scan_rels(&settings, vec![String::new()]).await?;
        self.sweep_temp_files().await;
        self.mark_index();
        Ok(())
    }

    /// Rescans the given relative paths; `""` means everything.
    pub(crate) async fn on_local_batch(&self, rels: Vec<String>) {
        let settings = self.settings();
        if settings.paused || self.is_stopped() {
            return;
        }
        if !settings.folder.is_dir() {
            self.error("the sync folder is missing".to_string()).await;
            return;
        }
        if let Err(e) = self.scan_rels(&settings, rels).await {
            self.error(format!("scanning: {e:#}")).await;
        }
        self.publish_state().await;
    }

    /// The four scan phases, holding the lock only for the two that touch
    /// the index. A folder switched underneath the scan discards it.
    async fn scan_rels(&self, settings: &Settings, rels: Vec<String>) -> Result<()> {
        let me = self.shared.identity.id.clone();
        let walked = walk_paths(&settings.folder, &rels).await?;
        let to_hash = {
            let mut inner = self.lock().await;
            if self.settings().folder != settings.folder {
                return Ok(());
            }
            let now = now_ms();
            let (out, to_hash) = apply_walk(&mut inner.index, &walked, &me, now);
            for dir in &walked.unreadable {
                if inner.unreadable.insert(dir.clone()) {
                    inner.push_error(format!(
                        "cannot read {dir}; its contents are left as they are"
                    ));
                }
            }
            self.absorb_local_changes(&mut inner, out, now);
            to_hash
        };
        if to_hash.is_empty() {
            return Ok(());
        }
        let hashed = hash_files(&settings.folder, to_hash).await;
        let mut inner = self.lock().await;
        if self.settings().folder != settings.folder {
            return Ok(());
        }
        let now = now_ms();
        let mut out = ScanOutcome::default();
        apply_hashes(&mut inner.index, hashed, &me, now, &mut out);
        self.absorb_local_changes(&mut inner, out, now);
        Ok(())
    }

    /// Records a scan's outcome: remembers what changed, tells every peer
    /// and the interface.
    pub(crate) fn absorb_local_changes(&self, inner: &mut Inner, out: ScanOutcome, now: i64) {
        for dir in &out.dirs_touched {
            self.dir_changed(dir);
        }
        if out.changed.is_empty() {
            return;
        }
        for e in &out.changed {
            inner.recent_changes.insert(e.path.clone(), now);
        }
        inner.last_change_ms = Some(now);
        self.broadcast_update(inner, out.changed);
        self.mark_index();
        self.mark_state();
    }

    pub(crate) fn broadcast_update(&self, inner: &Inner, entries: Vec<Entry>) {
        if entries.is_empty() {
            return;
        }
        for chunk in chunk_entries(entries) {
            let frame = Frame::Control(Control::IndexUpdate { entries: chunk });
            for link in inner.conns.values() {
                let _ = link.conn.try_send(frame.clone());
            }
        }
    }

    /// Removes `.owl-tmp-*` files older than an hour: a download aborted
    /// by a dropped link leaves one behind, ignored by scans but taking
    /// space.
    pub(crate) async fn sweep_temp_files(&self) {
        let settings = self.settings();
        if settings.paused || self.is_stopped() {
            return;
        }
        let removed = tokio::task::spawn_blocking(move || sweep_temp_files(&settings.folder))
            .await
            .unwrap_or(0);
        if removed > 0 {
            debug!("removed {removed} stale temporary files");
        }
    }

    fn status_for(
        &self,
        inner: &Inner,
        name: &str,
        path: &str,
        is_dir: bool,
        connected: bool,
        now: i64,
    ) -> EntryStatus {
        if is_conflict_name(name) {
            return EntryStatus::Conflict;
        }
        if let Some(progress) = self
            .shared
            .transfers
            .lock()
            .expect("transfers lock")
            .progress_for(path)
        {
            return EntryStatus::Syncing { progress };
        }
        if !connected {
            return EntryStatus::Local;
        }
        if !is_dir {
            let recent = inner
                .recent_changes
                .get(path)
                .is_some_and(|t| now - t < RECENT_MS);
            if recent || inner.queued_paths.contains(path) {
                return EntryStatus::Waiting;
            }
        }
        EntryStatus::Synced
    }
}

pub(crate) async fn list_dir(engine: &Engine, rel: &str) -> Result<Vec<DirEntry>> {
    let dir = engine.absolute_path(rel)?;
    let mut read = tokio::fs::read_dir(&dir)
        .await
        .with_context(|| format!("listing {}", dir.display()))?;
    let mut raw = Vec::new();
    while let Some(entry) = read.next_entry().await? {
        let name: String = entry.file_name().to_string_lossy().nfc().collect();
        let path = join_rel(rel, &name);
        if is_ignored(&path) {
            continue;
        }
        let Ok(md) = entry.metadata().await else {
            continue;
        };
        if !(md.is_dir() || md.is_file()) {
            continue;
        }
        raw.push((name, path, md.is_dir(), md.len(), mtime_ms(&md)));
    }

    let inner = engine.lock().await;
    let connected = !inner.conns.is_empty();
    let now = now_ms();
    let mut entries: Vec<DirEntry> = raw
        .into_iter()
        .map(|(name, path, is_dir, size, mtime_ms)| {
            let status = engine.status_for(&inner, &name, &path, is_dir, connected, now);
            DirEntry {
                name,
                path,
                is_dir,
                size: if is_dir { 0 } else { size },
                mtime_ms,
                status,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// Temporary files older than this are stale.
const TEMP_FILE_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

/// Deletes stale `.owl-tmp-*` files under `folder`, never following links
/// and never entering the engine's own metadata directory. Returns how
/// many were removed.
pub(crate) fn sweep_temp_files(folder: &Path) -> u32 {
    let mut removed = 0;
    let walker = walkdir::WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || e.file_name() != ".owl")
        .filter_map(|e| e.ok());
    for entry in walker {
        if !entry.file_type().is_file()
            || !entry.file_name().to_string_lossy().starts_with(".owl-tmp-")
        {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .and_then(|md| md.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > TEMP_FILE_TTL);
        if stale && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn import_tmp(dir: &Path) -> PathBuf {
    let suffix: u32 = rand::random();
    dir.join(format!(".owl-tmp-import-{suffix:08x}"))
}

/// Copies one file through a temporary name so the watcher never sees a
/// half file under its final name.
async fn copy_in(source: &Path, dest: &Path) -> Result<()> {
    let parent = dest.parent().context("destination has a parent")?;
    tokio::fs::create_dir_all(parent).await?;
    let tmp = import_tmp(parent);
    if let Err(e) = tokio::fs::copy(source, &tmp).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e).with_context(|| format!("copying {}", source.display()));
    }
    tokio::fs::rename(&tmp, dest)
        .await
        .with_context(|| format!("placing {}", dest.display()))
}

fn walk_source(source: &Path) -> Vec<(PathBuf, PathBuf)> {
    walkdir::WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(source).ok()?.to_path_buf();
            Some((e.path().to_path_buf(), rel))
        })
        .collect()
}

pub(crate) async fn import_files(
    engine: &Engine,
    sources: Vec<PathBuf>,
    into: &str,
) -> Result<u32> {
    let dest_dir = engine.absolute_path(into)?;
    tokio::fs::create_dir_all(&dest_dir).await?;
    let mut count = 0u32;
    for source in sources {
        let name = source
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.nfc().collect::<String>())
            .with_context(|| format!("{} has no file name", source.display()))?;
        validate_name(&name)?;
        if is_ignored(&name) {
            bail!("{name} is a reserved name");
        }
        let md = tokio::fs::metadata(&source)
            .await
            .with_context(|| format!("reading {}", source.display()))?;
        if md.is_dir() {
            let files = {
                let source = source.clone();
                tokio::task::spawn_blocking(move || walk_source(&source)).await?
            };
            for (abs, rel) in files {
                let rel_str = rel.to_string_lossy();
                if is_ignored(&rel_str.replace('\\', "/")) {
                    continue;
                }
                copy_in(&abs, &dest_dir.join(&name).join(rel)).await?;
                count += 1;
            }
        } else {
            copy_in(&source, &dest_dir.join(&name)).await?;
            count += 1;
        }
    }
    engine.on_local_batch(vec![into.to_string()]).await;
    Ok(count)
}

pub(crate) async fn import_reader<R: AsyncRead + Unpin + Send + 'static>(
    engine: &Engine,
    name: &str,
    into: &str,
    mut reader: R,
) -> Result<()> {
    let name: String = name.nfc().collect();
    validate_name(&name)?;
    if is_ignored(&name) {
        bail!("{name} is a reserved name");
    }
    let dest_dir = engine.absolute_path(into)?;
    tokio::fs::create_dir_all(&dest_dir).await?;
    let tmp = import_tmp(&dest_dir);
    let written = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        tokio::io::copy(&mut reader, &mut file).await?;
        file.flush().await?;
        Ok::<_, std::io::Error>(())
    }
    .await;
    if let Err(e) = written {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e).with_context(|| format!("writing {name}"));
    }
    tokio::fs::rename(&tmp, dest_dir.join(&name))
        .await
        .with_context(|| format!("placing {name}"))?;
    engine.on_local_batch(vec![join_rel(into, &name)]).await;
    Ok(())
}

pub(crate) async fn create_folder(engine: &Engine, rel: &str) -> Result<()> {
    validate_rel(rel)?;
    validate_name(file_name_of(rel))?;
    if is_ignored(rel) {
        bail!("{rel} is a reserved name");
    }
    let abs = engine.absolute_path(rel)?;
    tokio::fs::create_dir_all(&abs)
        .await
        .with_context(|| format!("creating {}", abs.display()))?;
    engine.on_local_batch(vec![rel.to_string()]).await;
    Ok(())
}

pub(crate) async fn delete_entry(engine: &Engine, rel: &str) -> Result<()> {
    validate_rel(rel)?;
    let abs = engine.absolute_path(rel)?;
    let md = tokio::fs::symlink_metadata(&abs)
        .await
        .with_context(|| format!("{rel} does not exist"))?;
    if md.is_dir() {
        tokio::fs::remove_dir_all(&abs).await
    } else {
        tokio::fs::remove_file(&abs).await
    }
    .with_context(|| format!("deleting {rel}"))?;
    engine.on_local_batch(vec![rel.to_string()]).await;
    Ok(())
}

pub(crate) async fn rename_entry(engine: &Engine, rel: &str, new_name: &str) -> Result<()> {
    validate_rel(rel)?;
    let new_name: String = new_name.trim().nfc().collect();
    validate_name(&new_name)?;
    if is_ignored(&new_name) {
        bail!("that name is reserved");
    }
    let new_rel = join_rel(parent_of(rel), &new_name);
    if new_rel == rel {
        return Ok(());
    }
    let from = engine.absolute_path(rel)?;
    let to = engine.absolute_path(&new_rel)?;
    // On a case-insensitive filesystem the target of a case-only rename
    // is the file itself, which is fine.
    let case_only = file_name_of(rel).to_lowercase() == new_name.to_lowercase();
    if !case_only && tokio::fs::symlink_metadata(&to).await.is_ok() {
        bail!("{new_name} already exists");
    }
    tokio::fs::rename(&from, &to)
        .await
        .with_context(|| format!("renaming {} to {new_name}", file_name_of(rel)))?;
    engine.on_local_batch(vec![rel.to_string(), new_rel]).await;
    Ok(())
}

pub(crate) async fn set_folder(engine: &Engine, path: PathBuf) -> Result<()> {
    tokio::fs::create_dir_all(&path)
        .await
        .with_context(|| format!("creating {}", path.display()))?;
    // Counters from the old index would make fresh entries look older than
    // whatever the peer holds and let it overwrite them. Starting every
    // counter past the largest one ever used makes them concurrent instead,
    // so a differing file becomes a conflict copy rather than being lost.
    let floor = {
        let mut inner = engine.lock().await;
        let floor = inner
            .index
            .entries()
            .flat_map(|e| e.vv.values().copied())
            .max()
            .unwrap_or(0);
        inner.watcher = None;
        // Downloads in flight belong to the old folder; the dial loop
        // reconnects within seconds and the new index is exchanged then.
        inner.conns.clear();
        inner.index.clear();
        inner.recent_changes.clear();
        inner.queued_paths.clear();
        inner.unreadable.clear();
        Index::delete_file(&engine.shared.data_dir)?;
        floor
    };
    engine
        .shared
        .settings
        .write()
        .expect("settings lock")
        .folder = path;
    if !engine.settings().paused {
        engine.start_folder().await?;
        let mut inner = engine.lock().await;
        let me = engine.shared.identity.id.clone();
        let raised: Vec<Entry> = inner
            .index
            .entries()
            .map(|e| {
                let mut e = e.clone();
                let mut vv = VersionVector::new();
                vv.insert(me.clone(), floor + 1);
                e.vv = vv;
                e
            })
            .collect();
        for e in raised {
            inner.index.insert(e);
        }
        let links: Vec<_> = inner.conns.values().map(|l| l.conn.clone()).collect();
        for conn in links {
            engine.send_full_index(&inner, &conn);
        }
        debug!("switched folder; {} entries indexed", inner.index.len());
    }
    engine.mark_index();
    engine.dir_changed("");
    engine.publish_state().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_removes_only_stale_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        let old = dir.path().join("sub/.owl-tmp-abcd-00000001");
        let fresh = dir.path().join(".owl-tmp-abcd-00000002");
        let real = dir.path().join("keep.txt");
        std::fs::write(&old, b"x").unwrap();
        std::fs::write(&fresh, b"x").unwrap();
        std::fs::write(&real, b"x").unwrap();
        crate::clock::set_mtime_ms(&old, crate::clock::now_ms() - 2 * 3600 * 1000).unwrap();
        crate::clock::set_mtime_ms(&real, crate::clock::now_ms() - 2 * 3600 * 1000).unwrap();
        assert_eq!(sweep_temp_files(dir.path()), 1);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(real.exists());
    }
}
