//! The file watcher: native events (inotify, ReadDirectoryChangesW) and an
//! optional poller, mapped to relative paths and debounced into batches.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{
    Config, Event, EventKind, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher as _,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::ignore::is_ignored;
use crate::paths::normalize_rel;

/// A batch is emitted after this much quiet.
const QUIET: Duration = Duration::from_millis(300);
/// A steady stream of events still yields a batch this often.
const MAX_WAIT: Duration = Duration::from_millis(1000);
const POLL_INTERVAL: Duration = Duration::from_secs(2);

pub struct Watcher {
    _native: RecommendedWatcher,
    _poll: Option<PollWatcher>,
    task: JoinHandle<()>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Watcher {
    /// Watches `root` recursively. Batches of relative paths arrive on `tx`;
    /// a batch containing `""` asks for a full rescan.
    pub fn start(root: PathBuf, poll: bool, tx: mpsc::Sender<Vec<String>>) -> Result<Watcher> {
        let (raw_tx, raw_rx) = mpsc::unbounded_channel::<notify::Result<Event>>();

        let handler = {
            let raw_tx = raw_tx.clone();
            move |res: notify::Result<Event>| {
                let _ = raw_tx.send(res);
            }
        };
        let mut native =
            RecommendedWatcher::new(handler, Config::default()).context("starting the watcher")?;
        native
            .watch(&root, RecursiveMode::Recursive)
            .with_context(|| format!("watching {}", root.display()))?;

        let poll_watcher = if poll {
            let handler = move |res: notify::Result<Event>| {
                let _ = raw_tx.send(res);
            };
            let mut w =
                PollWatcher::new(handler, Config::default().with_poll_interval(POLL_INTERVAL))
                    .context("starting the poll watcher")?;
            w.watch(&root, RecursiveMode::Recursive)
                .with_context(|| format!("polling {}", root.display()))?;
            Some(w)
        } else {
            None
        };

        // Events may carry either spelling of the root when it goes through
        // a symbolic link, so keep both.
        let canonical = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        let task = tokio::spawn(debounce_loop(root, canonical, raw_rx, tx));
        Ok(Watcher {
            _native: native,
            _poll: poll_watcher,
            task,
        })
    }
}

async fn debounce_loop(
    root: PathBuf,
    canonical: PathBuf,
    mut raw_rx: mpsc::UnboundedReceiver<notify::Result<Event>>,
    tx: mpsc::Sender<Vec<String>>,
) {
    loop {
        let Some(first) = raw_rx.recv().await else {
            return;
        };
        let started = Instant::now();
        let mut batch = BTreeSet::new();
        absorb(first, &root, &canonical, &mut batch);
        loop {
            let elapsed = started.elapsed();
            if elapsed >= MAX_WAIT {
                break;
            }
            let wait = QUIET.min(MAX_WAIT - elapsed);
            match tokio::time::timeout(wait, raw_rx.recv()).await {
                Ok(Some(ev)) => absorb(ev, &root, &canonical, &mut batch),
                Ok(None) => {
                    let _ = tx.send(batch.into_iter().collect()).await;
                    return;
                }
                Err(_) => break,
            }
        }
        if !batch.is_empty() && tx.send(batch.into_iter().collect()).await.is_err() {
            return;
        }
    }
}

fn absorb(res: notify::Result<Event>, root: &Path, canonical: &Path, batch: &mut BTreeSet<String>) {
    let event = match res {
        Ok(ev) => ev,
        Err(e) => {
            // A watcher error can mean lost events; a full rescan is the
            // only safe answer.
            warn!("watcher error, rescanning: {e}");
            batch.insert(String::new());
            return;
        }
    };
    if event.need_rescan() {
        batch.insert(String::new());
        return;
    }
    // Reads are noise; a close after writing is the one access event that
    // says something finished.
    if let EventKind::Access(access) = &event.kind {
        if !matches!(
            access,
            notify::event::AccessKind::Close(notify::event::AccessMode::Write)
        ) {
            return;
        }
    }
    for path in &event.paths {
        let rel = normalize_rel(path, root).or_else(|| normalize_rel(path, canonical));
        match rel {
            Some(r) if r.is_empty() => {
                batch.insert(String::new());
            }
            Some(r) if is_ignored(&r) => {}
            Some(r) => {
                batch.insert(r);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_is_reported_within_a_second() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let _watcher = Watcher::start(dir.path().to_path_buf(), false, tx).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        std::fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        let batch = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("a batch within two seconds")
            .expect("the watcher is alive");
        assert!(
            batch.iter().any(|p| p == "a.txt" || p.is_empty()),
            "batch was {batch:?}"
        );
    }

    #[tokio::test]
    async fn temp_files_are_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let _watcher = Watcher::start(dir.path().to_path_buf(), false, tx).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        std::fs::write(dir.path().join(".owl-tmp-abc"), b"partial").unwrap();
        std::fs::write(dir.path().join("real.txt"), b"hello").unwrap();
        let batch = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(!batch.iter().any(|p| p.starts_with(".owl-tmp-")));
        assert!(batch.iter().any(|p| p == "real.txt"));
    }
}
