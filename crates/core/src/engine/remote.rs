//! Remote changes: what to do with entries a peer announces, fetching the
//! bytes when needed, and putting a fetched file in place.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{debug, warn};

use super::inner::{Inner, Settings};
use super::Engine;
use crate::clock::{mtime_ms, now_ms, set_mtime_ms};
use crate::hash::is_hex_hash;
use crate::ignore::is_ignored;
use crate::index::{Entry, EntryKind};
use crate::paths::{conflict_name, parent_of, safe_abs, validate_rel};
use crate::sync::{apply_adopt, decide, needs_bytes, resolve_conflict, Decision};
use crate::transfer::{self, Requester};

/// Whether an entry from the wire is well formed: a valid relative path
/// that is not ignored, a lowercase hex hash on live files and no hash
/// anywhere else. The hash is the only wire string besides the path that
/// reaches a file name, so it is checked before anything is done with it.
fn acceptable(remote: &Entry) -> bool {
    if validate_rel(&remote.path).is_err() || is_ignored(&remote.path) {
        return false;
    }
    if remote.deleted || remote.kind == EntryKind::Dir {
        remote.hash.is_none()
    } else {
        remote.hash.as_deref().is_some_and(is_hex_hash)
    }
}

impl Engine {
    /// Handles one batch of entries from a peer. Live entries go first so a
    /// rename (a new path plus a tombstone) can copy the old file before it
    /// is removed. `link_id` names the connection the batch came in on;
    /// frames still buffered from a peer that was forgotten or replaced
    /// since are dropped.
    pub(crate) async fn on_remote_entries(&self, peer_id: &str, link_id: u64, entries: Vec<Entry>) {
        let settings = self.settings();
        if settings.paused || self.is_stopped() {
            return;
        }
        let me = self.shared.identity.id.clone();
        let mut inner = self.lock().await;
        if !inner.is_linked(peer_id, link_id) {
            debug!(
                "dropping {} entries from a stale connection to {peer_id}",
                entries.len()
            );
            return;
        }
        let now = now_ms();

        let (mut live, mut tombs): (Vec<Entry>, Vec<Entry>) =
            entries.into_iter().partition(|e| !e.deleted);
        live.sort_by(|a, b| {
            (a.kind != EntryKind::Dir)
                .cmp(&(b.kind != EntryKind::Dir))
                .then_with(|| a.path.len().cmp(&b.path.len()))
        });
        // Deepest first, so a directory is empty by the time its own
        // tombstone is applied.
        tombs.sort_by(|a, b| b.path.len().cmp(&a.path.len()));

        let mut announce = Vec::new();
        for remote in live.into_iter().chain(tombs) {
            if !acceptable(&remote) {
                warn!(
                    "{peer_id} sent a malformed entry for {:?}; dropped",
                    remote.path
                );
                continue;
            }
            let local = inner.index.get(&remote.path).cloned();
            match decide(local.as_ref(), &remote, &me, peer_id) {
                Decision::Ignore => {}
                Decision::Adopt => {
                    self.adopt(
                        &mut inner,
                        peer_id,
                        link_id,
                        &settings,
                        local.as_ref(),
                        remote,
                        now,
                        &mut announce,
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
                        self.dir_over_file(
                            &mut inner,
                            &settings,
                            &local,
                            &remote,
                            now,
                            &mut announce,
                        )
                        .await;
                    } else {
                        self.want_bytes(&mut inner, peer_id, &settings, remote, now)
                            .await;
                    }
                }
            }
        }
        if !announce.is_empty() {
            inner.last_change_ms = Some(now);
            self.broadcast_update(&inner, announce);
        }
        self.mark_index();
        drop(inner);
        self.mark_state();
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
            self.want_bytes(inner, peer_id, settings, remote, now).await;
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
        inner.recent_changes.insert(winner.path.clone(), now);
        announce.push(winner);
        self.dir_changed(parent_of(&remote.path));
    }

    /// Gets a remote file's bytes: by copying a local file with the same
    /// content when there is one, which makes a rename on the other side
    /// instant here, otherwise by queueing a download from the peer.
    async fn want_bytes(
        &self,
        inner: &mut Inner,
        peer_id: &str,
        settings: &Settings,
        remote: Entry,
        now: i64,
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
        if let Some(source) = source {
            match transfer::local_copy(&settings.folder, &source, &remote).await {
                Ok(Some(tmp)) => {
                    self.install_file(inner, peer_id, settings, remote, tmp, now)
                        .await;
                    return;
                }
                Ok(None) => {}
                Err(e) => debug!("local copy of {source} failed: {e:#}"),
            }
        }
        if let Some(link) = inner.conns.get(peer_id) {
            if link.queue.send(remote.clone()).is_ok() {
                inner.queued_paths.insert(remote.path);
                self.shared
                    .transfers
                    .lock()
                    .expect("transfers lock")
                    .adjust_queued(1);
            }
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
        let mut inner = self.lock().await;
        self.install_file(&mut inner, &peer_id, &settings, entry, tmp, now_ms())
            .await;
        Ok(())
    }

    /// Puts a fetched file in place, re-checking the decision now that the
    /// bytes are here. On a conflict the local file moves aside first, so
    /// there is never a moment where the path is missing.
    pub(crate) async fn install_file(
        &self,
        inner: &mut Inner,
        peer_id: &str,
        settings: &Settings,
        remote: Entry,
        tmp: PathBuf,
        now: i64,
    ) {
        let me = &self.shared.identity.id;
        let folder = &settings.folder;
        let dest = match safe_abs(folder, &remote.path) {
            Ok(dest) => dest,
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                inner.push_error(format!("refusing {}: {e:#}", remote.path));
                return;
            }
        };
        let local = inner.index.get(&remote.path).cloned();
        let mut announce = Vec::new();
        match decide(local.as_ref(), &remote, me, peer_id) {
            Decision::Adopt if needs_bytes(local.as_ref(), &remote) => {
                if let Err(e) = place(&tmp, &dest, &settings.device_name, now).await {
                    inner.push_error(format!("placing {}: {e:#}", remote.path));
                    return;
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
                    return;
                }
                inner.recent_changes.insert(winner.path.clone(), now);
                announce.push(winner);
            }
            _ => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return;
            }
        }
        inner.last_change_ms = Some(now);
        self.dir_changed(parent_of(&remote.path));
        if !announce.is_empty() {
            self.broadcast_update(inner, announce);
        }
        self.mark_index();
        self.mark_state();
    }
}

/// Moves a fetched file into place. A directory in the way is moved aside
/// under a conflict name rather than deleted; the watcher indexes it.
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
