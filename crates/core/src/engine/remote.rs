//! Remote changes: what to do with entries a peer announces, fetching the
//! bytes when needed, and putting a fetched file in place.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use tracing::{debug, warn};
use unicode_normalization::UnicodeNormalization;

use super::inner::{Inner, Settings};
use super::Engine;
use crate::clock::{mtime_ms, now_ms, set_mtime_ms};
use crate::hash::is_hex_hash;
use crate::ignore::is_ignored;
use crate::index::{Entry, EntryKind, Index};
use crate::paths::{conflict_name, join_rel, parent_of, safe_abs, validate_portable, validate_rel};
use crate::sync::{apply_adopt, decide, needs_bytes, resolve_conflict, Decision};
use crate::transfer::{self, Requester};
use crate::vv::VersionVector;

/// Why an entry from the wire was not applied.
enum Rejected {
    /// Malformed: a bad path, an ignored name, or a hash that is not a
    /// lowercase hex digest where one is required (or present where none
    /// belongs). Logged, never shown: only a broken peer sends these.
    Malformed,
    /// A name this device cannot store; shown once, so the person can
    /// rename it on the other side.
    Unportable(String),
}

/// The hash is the only wire string besides the path that reaches a file
/// name, so it is checked before anything is done with it.
fn check(remote: &Entry) -> Result<(), Rejected> {
    if validate_rel(&remote.path).is_err() || is_ignored(&remote.path) {
        return Err(Rejected::Malformed);
    }
    let hash_ok = if remote.deleted || remote.kind == EntryKind::Dir {
        remote.hash.is_none()
    } else {
        remote.hash.as_deref().is_some_and(is_hex_hash)
    };
    if !hash_ok {
        return Err(Rejected::Malformed);
    }
    if let Err(e) = validate_portable(&remote.path) {
        return Err(Rejected::Unportable(format!(
            "cannot sync {}: {e}",
            remote.path
        )));
    }
    Ok(())
}

/// A same-content local file found for a remote entry: copying it makes a
/// rename on the other side instant here. The copy runs outside the lock.
struct CopyJob {
    remote: Entry,
    source: String,
}

impl Engine {
    /// Handles one batch of entries from a peer in two passes around the
    /// copies of same-content files, so the engine lock is never held
    /// while a large file is copied: live entries are decided under the
    /// lock, copies run without it, then copies are installed and
    /// tombstones applied under the lock again. Tombstones go last so a
    /// rename (a new path plus a tombstone) still finds its source.
    /// `link_id` names the connection the batch came in on; frames still
    /// buffered from a peer that was forgotten or replaced since are
    /// dropped.
    pub(crate) async fn on_remote_entries(&self, peer_id: &str, link_id: u64, entries: Vec<Entry>) {
        let settings = self.settings();
        if settings.paused || self.is_stopped() {
            return;
        }
        let mut live = Vec::new();
        let mut tombs = Vec::new();
        let mut reported: HashSet<String> = HashSet::new();
        let mut unportable = Vec::new();
        for mut remote in entries {
            remote.path = remote.path.nfc().collect();
            match check(&remote) {
                Ok(()) => {
                    if remote.deleted {
                        tombs.push(remote);
                    } else {
                        live.push(remote);
                    }
                }
                Err(Rejected::Malformed) => {
                    warn!(
                        "{peer_id} sent a malformed entry for {:?}; dropped",
                        remote.path
                    );
                }
                Err(Rejected::Unportable(message)) => {
                    if reported.insert(remote.path.clone()) {
                        unportable.push(message);
                    }
                }
            }
        }
        // Directories before the files inside them, shorter paths first;
        // tombstones deepest first, so a directory is empty by the time
        // its own tombstone is applied.
        live.sort_by(|a, b| {
            (a.kind != EntryKind::Dir)
                .cmp(&(b.kind != EntryKind::Dir))
                .then_with(|| a.path.len().cmp(&b.path.len()))
        });
        tombs.sort_by(|a, b| b.path.len().cmp(&a.path.len()));

        let mut copies = Vec::new();
        {
            let mut inner = self.lock().await;
            if !inner.is_linked(peer_id, link_id) {
                debug!("dropping a batch from a stale connection to {peer_id}");
                return;
            }
            for message in unportable {
                inner.push_error(message);
            }
            let now = now_ms();
            let mut announce = Vec::new();
            for remote in live {
                self.handle_entry(
                    &mut inner,
                    peer_id,
                    link_id,
                    &settings,
                    remote,
                    now,
                    &mut announce,
                    &mut copies,
                )
                .await;
            }
            self.finish_batch(&mut inner, announce, now);
        }

        let mut installs = Vec::new();
        let mut downloads = Vec::new();
        for job in copies {
            match transfer::local_copy(&settings.folder, &job.source, &job.remote).await {
                Ok(Some(tmp)) => installs.push((job.remote, tmp)),
                Ok(None) => downloads.push(job.remote),
                Err(e) => {
                    debug!("local copy of {} failed: {e:#}", job.source);
                    downloads.push(job.remote);
                }
            }
        }

        {
            let mut inner = self.lock().await;
            if !inner.is_linked(peer_id, link_id) {
                for (_, tmp) in installs {
                    let _ = tokio::fs::remove_file(&tmp).await;
                }
                return;
            }
            let now = now_ms();
            let mut announce = Vec::new();
            for (remote, tmp) in installs {
                if let Err(e) = self
                    .install_file(&mut inner, peer_id, &settings, remote, tmp, now)
                    .await
                {
                    debug!("local copy not installed: {e:#}");
                }
            }
            for remote in downloads {
                self.queue_download(&mut inner, peer_id, remote);
            }
            let mut unused = Vec::new();
            for remote in tombs {
                self.handle_entry(
                    &mut inner,
                    peer_id,
                    link_id,
                    &settings,
                    remote,
                    now,
                    &mut announce,
                    &mut unused,
                )
                .await;
            }
            self.finish_batch(&mut inner, announce, now);
        }
        self.mark_state();
    }

    fn finish_batch(&self, inner: &mut Inner, announce: Vec<Entry>, now: i64) {
        if !announce.is_empty() {
            inner.last_change_ms = Some(now);
            self.broadcast_update(inner, announce);
        }
        self.mark_index();
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_entry(
        &self,
        inner: &mut Inner,
        peer_id: &str,
        link_id: u64,
        settings: &Settings,
        remote: Entry,
        now: i64,
        announce: &mut Vec<Entry>,
        copies: &mut Vec<CopyJob>,
    ) {
        let me = self.shared.identity.id.clone();
        let local = inner.index.get(&remote.path).cloned();
        match decide(local.as_ref(), &remote, &me, peer_id) {
            Decision::Ignore => {}
            Decision::Adopt => {
                self.adopt(
                    inner,
                    peer_id,
                    link_id,
                    settings,
                    local.as_ref(),
                    remote,
                    now,
                    announce,
                    copies,
                )
                .await
            }
            Decision::Conflict {
                winner_remote: false,
            } => {
                let local = local.expect("a conflict has a local entry");
                let (winner, _) = resolve_conflict(
                    &mut inner.index,
                    &local,
                    &remote,
                    false,
                    &me,
                    &settings.device_name,
                    now,
                );
                inner.recent_changes.insert(winner.path.clone(), now);
                announce.push(winner);
            }
            Decision::Conflict {
                winner_remote: true,
            } => {
                if remote.kind == EntryKind::Dir {
                    let local = local.expect("a conflict has a local entry");
                    self.dir_over_file(inner, settings, &local, &remote, now, announce)
                        .await;
                } else {
                    self.want_bytes(inner, peer_id, settings, remote, copies);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn adopt(
        &self,
        inner: &mut Inner,
        peer_id: &str,
        link_id: u64,
        settings: &Settings,
        local: Option<&Entry>,
        remote: Entry,
        now: i64,
        announce: &mut Vec<Entry>,
        copies: &mut Vec<CopyJob>,
    ) {
        let me = &self.shared.identity.id;
        let folder = &settings.folder;
        if remote.deleted {
            if let Some(l) = local.filter(|l| !l.deleted) {
                if !delete_local(folder, l).await {
                    if l.kind == EntryKind::Dir && inner.tombstone_retries.insert(l.path.clone()) {
                        // Its children may be in a later chunk; look again
                        // once they have had a chance to arrive.
                        let _ = self.shared.retry_tx.send((
                            peer_id.to_string(),
                            link_id,
                            remote.clone(),
                        ));
                        return;
                    }
                    inner.tombstone_retries.remove(&l.path);
                    // Still there, or edited since the last scan: keep ours
                    // and let the peer take it back. A rescan picks up any
                    // unscanned edit.
                    let (winner, _) = resolve_conflict(
                        &mut inner.index,
                        l,
                        &remote,
                        false,
                        me,
                        &settings.device_name,
                        now,
                    );
                    announce.push(winner);
                    let _ = self.shared.watch_tx.try_send(vec![l.path.clone()]);
                    return;
                }
                inner.tombstone_retries.remove(&l.path);
                self.dir_changed(parent_of(&l.path));
                if l.kind == EntryKind::Dir {
                    self.dir_changed(&l.path);
                }
            }
            apply_adopt(&mut inner.index, &remote, now);
            inner.last_change_ms = Some(now);
        } else if remote.kind == EntryKind::Dir {
            let abs = match safe_abs(folder, &remote.path) {
                Ok(abs) => abs,
                Err(e) => {
                    inner.push_error(format!("refusing {}: {e:#}", remote.path));
                    return;
                }
            };
            if let Some(l) = local.filter(|l| l.is_live_file()) {
                if !delete_local(folder, l).await {
                    let _ = self.shared.watch_tx.try_send(vec![l.path.clone()]);
                    return;
                }
            }
            if let Err(e) = tokio::fs::create_dir_all(&abs).await {
                inner.push_error(format!("creating {}: {e}", remote.path));
                return;
            }
            record_ancestors(&mut inner.index, folder, &remote.path, now);
            apply_adopt(&mut inner.index, &remote, now);
            inner.last_change_ms = Some(now);
            self.dir_changed(parent_of(&remote.path));
        } else if !needs_bytes(local, &remote) {
            // Same content already here: align the mtime, keep the merged
            // vector.
            if local.is_some_and(|l| l.mtime_ms != remote.mtime_ms) {
                if let Ok(abs) = safe_abs(folder, &remote.path) {
                    let _ = set_mtime_ms(&abs, remote.mtime_ms);
                }
            }
            apply_adopt(&mut inner.index, &remote, now);
        } else {
            self.want_bytes(inner, peer_id, settings, remote, copies);
        }
    }

    /// A remote directory wins over a local file of the same name: the file
    /// moves aside as a conflict copy and the directory is created.
    async fn dir_over_file(
        &self,
        inner: &mut Inner,
        settings: &Settings,
        local: &Entry,
        remote: &Entry,
        now: i64,
        announce: &mut Vec<Entry>,
    ) {
        let me = &self.shared.identity.id;
        let folder = &settings.folder;
        let (winner, loser) = resolve_conflict(
            &mut inner.index,
            local,
            remote,
            true,
            me,
            &settings.device_name,
            now,
        );
        let (from, to, dir) = match (
            safe_abs(folder, &local.path),
            loser.as_ref().map(|l| safe_abs(folder, &l.path)),
            safe_abs(folder, &remote.path),
        ) {
            (Ok(from), Some(Ok(to)), Ok(dir)) => (from, Some(to), dir),
            (Ok(from), None, Ok(dir)) => (from, None, dir),
            _ => {
                inner.push_error(format!(
                    "refusing {}: symbolic link in the path",
                    remote.path
                ));
                if let Some(l) = &loser {
                    inner.index.remove(&l.path);
                }
                inner.index.insert(local.clone());
                return;
            }
        };
        if let (Some(loser), Some(to)) = (loser, to) {
            match tokio::fs::rename(&from, &to).await {
                Ok(()) => {
                    inner.recent_changes.insert(loser.path.clone(), now);
                    announce.push(loser);
                }
                Err(_) => {
                    inner.index.remove(&loser.path);
                }
            }
        }
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            inner.push_error(format!("creating {}: {e}", remote.path));
            return;
        }
        record_ancestors(&mut inner.index, folder, &remote.path, now);
        inner.recent_changes.insert(winner.path.clone(), now);
        announce.push(winner);
        self.dir_changed(parent_of(&remote.path));
    }

    /// Gets a remote file's bytes: by copying a local file with the same
    /// content when there is one (recorded as a job for after the lock),
    /// otherwise by queueing a download from the peer.
    fn want_bytes(
        &self,
        inner: &mut Inner,
        peer_id: &str,
        settings: &Settings,
        remote: Entry,
        copies: &mut Vec<CopyJob>,
    ) {
        let Some(hash) = remote.hash.as_deref() else {
            return;
        };
        if let Err(e) = safe_abs(&settings.folder, &remote.path) {
            inner.push_error(format!("refusing {}: {e:#}", remote.path));
            return;
        }
        let source = inner
            .index
            .find_by_hash(hash)
            .filter(|e| e.path != remote.path)
            .map(|e| e.path.clone());
        match source {
            Some(source) => copies.push(CopyJob { remote, source }),
            None => self.queue_download(inner, peer_id, remote),
        }
    }

    fn queue_download(&self, inner: &mut Inner, peer_id: &str, remote: Entry) {
        let path = remote.path.clone();
        let queued = inner
            .conns
            .get(peer_id)
            .is_some_and(|link| link.enqueue(remote));
        if queued {
            inner.queued_paths.insert(path);
            self.shared
                .transfers
                .lock()
                .expect("transfers lock")
                .adjust_queued(1);
        }
    }

    /// The download worker's job for one queued entry.
    pub(crate) async fn fetch_entry(
        &self,
        requester: &Arc<Requester>,
        link_id: u64,
        entry: Entry,
    ) -> Result<()> {
        let me = self.shared.identity.id.clone();
        let peer_id = requester.conn().peer_id.clone();
        {
            let mut inner = self.lock().await;
            inner.queued_paths.remove(&entry.path);
            self.shared
                .transfers
                .lock()
                .expect("transfers lock")
                .adjust_queued(-1);
            if !inner.is_linked(&peer_id, link_id) {
                return Ok(());
            }
            let local = inner.index.get(&entry.path);
            let wanted = matches!(
                decide(local, &entry, &me, &peer_id),
                Decision::Adopt
                    | Decision::Conflict {
                        winner_remote: true
                    }
            ) && needs_bytes(local, &entry);
            if !wanted {
                return Ok(());
            }
        }
        self.mark_state();
        let settings = self.settings();
        if settings.paused || self.is_stopped() {
            return Ok(());
        }

        let hash = entry.hash.clone().context("a file entry carries a hash")?;
        let source = {
            let inner = self.lock().await;
            inner
                .index
                .find_by_hash(&hash)
                .filter(|e| e.path != entry.path)
                .map(|e| e.path.clone())
        };
        let mut tmp: Option<PathBuf> = None;
        if let Some(source) = source {
            tmp = transfer::local_copy(&settings.folder, &source, &entry)
                .await
                .ok()
                .flatten();
        }
        let tmp = match tmp {
            Some(t) => t,
            None => {
                transfer::download(
                    requester.clone(),
                    &settings.folder,
                    &entry,
                    self.shared.transfers.clone(),
                )
                .await?
            }
        };
        if self.settings().folder != settings.folder {
            // The folder was switched while the bytes were on their way.
            let _ = tokio::fs::remove_file(&tmp).await;
            return Ok(());
        }
        let mut inner = self.lock().await;
        self.install_file(&mut inner, &peer_id, &settings, entry, tmp, now_ms())
            .await
    }

    /// Puts a fetched file in place, re-checking the decision now that the
    /// bytes are here. On a conflict the local file moves aside first, so
    /// there is never a moment where the path is missing. What is on disk
    /// must still match the index entry the decision was based on: an
    /// edit the scanner has not seen yet is never overwritten; the path is
    /// rescanned and the error asks the caller to try again.
    pub(crate) async fn install_file(
        &self,
        inner: &mut Inner,
        peer_id: &str,
        settings: &Settings,
        remote: Entry,
        tmp: PathBuf,
        now: i64,
    ) -> Result<()> {
        let me = &self.shared.identity.id;
        let folder = &settings.folder;
        let dest = match safe_abs(folder, &remote.path) {
            Ok(dest) => dest,
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                inner.push_error(format!("refusing {}: {e:#}", remote.path));
                return Ok(());
            }
        };
        let local = inner.index.get(&remote.path).cloned();
        let decision = decide(local.as_ref(), &remote, me, peer_id);
        let wants_bytes = matches!(
            decision,
            Decision::Adopt
                | Decision::Conflict {
                    winner_remote: true
                }
        ) && needs_bytes(local.as_ref(), &remote);
        if wants_bytes && !disk_matches(&dest, local.as_ref()).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            let _ = self.shared.watch_tx.try_send(vec![remote.path.clone()]);
            return Err(anyhow!(
                "{} changed on disk since the last scan",
                remote.path
            ));
        }
        let mut announce = Vec::new();
        match decision {
            Decision::Adopt if !needs_bytes(local.as_ref(), &remote) => {
                // Same content arrived by another route meanwhile: nothing
                // to place, but the merged vector still counts.
                let _ = tokio::fs::remove_file(&tmp).await;
                if local
                    .as_ref()
                    .is_some_and(|l| l.mtime_ms != remote.mtime_ms)
                {
                    let _ = set_mtime_ms(&dest, remote.mtime_ms);
                }
                apply_adopt(&mut inner.index, &remote, now);
                self.mark_index();
                return Ok(());
            }
            Decision::Adopt => {
                record_ancestors(&mut inner.index, folder, &remote.path, now);
                if let Err(e) = place(&tmp, &dest, &settings.device_name, now).await {
                    inner.push_error(format!("placing {}: {e:#}", remote.path));
                    return Ok(());
                }
                apply_adopt(&mut inner.index, &remote, now);
            }
            Decision::Conflict {
                winner_remote: true,
            } => {
                let local = local.expect("a conflict has a local entry");
                let (winner, loser) = resolve_conflict(
                    &mut inner.index,
                    &local,
                    &remote,
                    true,
                    me,
                    &settings.device_name,
                    now,
                );
                if let Some(loser) = loser {
                    let aside = safe_abs(folder, &loser.path).unwrap_or_else(|_| dest.clone());
                    match tokio::fs::rename(&dest, &aside).await {
                        Ok(()) => {
                            inner.recent_changes.insert(loser.path.clone(), now);
                            announce.push(loser);
                        }
                        Err(_) => {
                            inner.index.remove(&loser.path);
                        }
                    }
                }
                if let Err(e) = place(&tmp, &dest, &settings.device_name, now).await {
                    inner.push_error(format!("placing {}: {e:#}", remote.path));
                    return Ok(());
                }
                inner.recent_changes.insert(winner.path.clone(), now);
                announce.push(winner);
            }
            _ => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Ok(());
            }
        }
        inner.last_change_ms = Some(now);
        self.dir_changed(parent_of(&remote.path));
        if !announce.is_empty() {
            self.broadcast_update(inner, announce);
        }
        self.mark_index();
        self.mark_state();
        Ok(())
    }
}

/// Directories created on the way to a remote path are recorded with an
/// empty version vector, so the watcher does not announce them as local
/// changes and any entry a peer holds for them wins.
fn record_ancestors(index: &mut Index, folder: &Path, path: &str, now: i64) {
    let mut prefix = String::new();
    for component in parent_of(path).split('/').filter(|c| !c.is_empty()) {
        prefix = join_rel(&prefix, component);
        if index.get(&prefix).is_some_and(|e| e.is_live_dir()) {
            continue;
        }
        let mtime = std::fs::metadata(folder.join(&prefix))
            .map(|md| mtime_ms(&md))
            .unwrap_or(now);
        index.insert(Entry {
            path: prefix.clone(),
            kind: EntryKind::Dir,
            size: 0,
            mtime_ms: mtime,
            hash: None,
            deleted: false,
            vv: VersionVector::new(),
            seen_at_ms: now,
        });
    }
}

/// Whether what is on disk at `dest` is what the index entry describes,
/// so replacing it cannot destroy an edit the scanner has not seen.
async fn disk_matches(dest: &Path, local: Option<&Entry>) -> bool {
    match tokio::fs::symlink_metadata(dest).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => local.is_none_or(|l| l.deleted),
        Err(_) => false,
        Ok(md) => match local {
            Some(l) if !l.deleted => match l.kind {
                EntryKind::File => {
                    md.is_file() && md.len() == l.size && mtime_ms(&md) == l.mtime_ms
                }
                EntryKind::Dir => md.is_dir(),
            },
            _ => false,
        },
    }
}

/// Moves a fetched file into place. A directory in the way is moved aside
/// under a conflict name rather than deleted; the watcher indexes it. The
/// caller has already recorded and created the parent directories.
async fn place(tmp: &Path, dest: &Path, device_name: &str, now: i64) -> Result<()> {
    if let Ok(md) = tokio::fs::symlink_metadata(dest).await {
        if md.is_dir() {
            let name = dest
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("folder");
            let aside = dest.with_file_name(conflict_name(name, device_name, now));
            tokio::fs::rename(dest, &aside)
                .await
                .with_context(|| format!("moving {} aside", dest.display()))?;
        }
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::rename(tmp, dest)
        .await
        .with_context(|| format!("renaming into {}", dest.display()))
}

/// Removes a local file or an empty directory that a tombstone covers.
/// Returns false if anything on disk differs from the index, in which case
/// nothing is removed: a deletion never destroys an unscanned edit.
async fn delete_local(folder: &Path, local: &Entry) -> bool {
    let Ok(abs) = safe_abs(folder, &local.path) else {
        return false;
    };
    match tokio::fs::symlink_metadata(&abs).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
        Ok(md) => match local.kind {
            EntryKind::File => {
                if !md.is_file() || md.len() != local.size || mtime_ms(&md) != local.mtime_ms {
                    return false;
                }
                tokio::fs::remove_file(&abs).await.is_ok()
            }
            EntryKind::Dir => md.is_dir() && tokio::fs::remove_dir(&abs).await.is_ok(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::set_mtime_ms;
    use std::collections::BTreeMap;

    fn file_entry(path: &str, size: u64, mtime_ms: i64, deleted: bool) -> Entry {
        Entry {
            path: path.into(),
            kind: EntryKind::File,
            size,
            mtime_ms,
            hash: Some("h".into()),
            deleted,
            vv: BTreeMap::new(),
            seen_at_ms: 0,
        }
    }

    #[tokio::test]
    async fn disk_matches_guards_unscanned_edits() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("doc.txt");
        assert!(
            disk_matches(&dest, None).await,
            "nothing there, nothing indexed"
        );
        assert!(disk_matches(&dest, Some(&file_entry("doc.txt", 0, 0, true))).await);
        assert!(
            !disk_matches(&dest, Some(&file_entry("doc.txt", 5, 1, false))).await,
            "indexed but gone: the person deleted it since the scan"
        );

        std::fs::write(&dest, b"hello").unwrap();
        set_mtime_ms(&dest, 1_700_000_000_000).unwrap();
        let indexed = file_entry("doc.txt", 5, 1_700_000_000_000, false);
        assert!(disk_matches(&dest, Some(&indexed)).await);
        assert!(
            !disk_matches(&dest, None).await,
            "a file the index does not know is never overwritten"
        );
        std::fs::write(&dest, b"hello, edited").unwrap();
        assert!(!disk_matches(&dest, Some(&indexed)).await);
    }

    #[test]
    fn ancestors_are_recorded_with_empty_vectors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        let mut index = Index::default();
        record_ancestors(&mut index, dir.path(), "a/b/c.txt", 7);
        let a = index.get("a").unwrap();
        assert!(a.is_live_dir());
        assert!(a.vv.is_empty());
        assert!(index.get("a/b").unwrap().is_live_dir());
        assert!(index.get("a/b/c.txt").is_none());
    }

    #[test]
    fn wire_entries_are_checked() {
        let mut ok = file_entry("a/b.txt", 1, 1, false);
        ok.hash = Some("a".repeat(64));
        assert!(check(&ok).is_ok());
        let mut bad_hash = ok.clone();
        bad_hash.hash = Some("../../../escape/".into());
        assert!(matches!(check(&bad_hash), Err(Rejected::Malformed)));
        let mut tomb_with_hash = ok.clone();
        tomb_with_hash.deleted = true;
        assert!(matches!(check(&tomb_with_hash), Err(Rejected::Malformed)));
        let mut dotdot = ok.clone();
        dotdot.path = "../x".into();
        assert!(matches!(check(&dotdot), Err(Rejected::Malformed)));
        let mut unportable = ok.clone();
        unportable.path = "a:b.txt".into();
        assert!(matches!(check(&unportable), Err(Rejected::Unportable(_))));
    }
}
