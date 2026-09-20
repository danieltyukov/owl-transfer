//! The engine: one value that owns the index, the peer connections and the
//! background tasks, publishes `State` snapshots, and exposes everything
//! the interface can do.
//!
//! Locking: `Inner` sits behind a tokio mutex and is the only place sync
//! state changes. Settings the interface reads synchronously live in a
//! std `RwLock` beside it. Transfer progress has its own std mutex because
//! download tasks update it without touching `Inner`.

mod inner;
mod local;
mod peer;
mod remote;
mod tasks;

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use anyhow::{bail, Context, Result};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc, watch, Mutex, Notify};
use tokio::task::JoinHandle;

use crate::beacon::Beacon;
use crate::clock::now_ms;
use crate::config::{Config, DeviceKind};
use crate::identity::Identity;
use crate::index::Index;
use crate::paths::validate_rel;
use crate::peers::PeerStore;
use crate::state::{DirEntry, PairingDirection, State};
use crate::transfer::{SharedTransfers, TransferState};

use inner::{Inner, Settings};

/// Tombstones older than this are dropped.
const TOMBSTONE_TTL_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(Clone)]
pub struct Engine {
    shared: Arc<Shared>,
}

pub(crate) struct Shared {
    pub(crate) identity: Identity,
    pub(crate) data_dir: PathBuf,
    pub(crate) kind: DeviceKind,
    pub(crate) poll_watch: bool,
    pub(crate) local_port: u16,
    pub(crate) settings: RwLock<Settings>,
    pub(crate) inner: Mutex<Inner>,
    pub(crate) trusted_ids: Arc<RwLock<HashSet<String>>>,
    pub(crate) transfers: SharedTransfers,
    pub(crate) state_tx: watch::Sender<State>,
    pub(crate) dir_tx: broadcast::Sender<String>,
    pub(crate) state_dirty: Notify,
    pub(crate) index_dirty: Notify,
    pub(crate) watch_tx: mpsc::Sender<Vec<String>>,
    /// Remote tombstones to apply again after a short delay: peer, link, entry.
    pub(crate) retry_tx: mpsc::UnboundedSender<(String, u64, crate::index::Entry)>,
    pub(crate) tasks: std::sync::Mutex<Vec<JoinHandle<()>>>,
    pub(crate) stopped: AtomicBool,
    /// Connections that have not yet proven a pairing.
    pub(crate) unauthenticated: AtomicUsize,
    /// Downloads a peer may have queued at once; the rest wait as deferred.
    pub(crate) queue_cap: AtomicUsize,
    /// A pause before the hash phase of a floor scan, so tests can make a
    /// peer reconnect in the middle of one. Zero in production.
    pub(crate) hash_delay_ms: std::sync::atomic::AtomicU64,
    /// Block requests to answer "unavailable" before serving again, so
    /// tests can make a peer's download fail once. Zero in production.
    pub(crate) fail_blocks: std::sync::atomic::AtomicU32,
    /// Temporary files between having their mtime set and being renamed
    /// into place; the stale-file sweep leaves them alone.
    pub(crate) tmp_in_flight: std::sync::Mutex<HashSet<PathBuf>>,
}

/// Downloads a peer may have queued at once.
const DEFAULT_QUEUE_CAP: usize = 10_000;

impl Engine {
    pub async fn start(config: Config) -> Result<Engine> {
        std::fs::create_dir_all(&config.data_dir)
            .with_context(|| format!("creating {}", config.data_dir.display()))?;
        // A paused start (Android before the storage grant) touches nothing
        // under the folder: identity, peers, beacon, listener and pairing
        // all work without it, and `set_paused(false)` creates it later.
        if !config.paused {
            std::fs::create_dir_all(&config.folder)
                .with_context(|| format!("creating {}", config.folder.display()))?;
        }
        let identity = Identity::load_or_create(&config.data_dir)?;
        let (peers, peers_note) = PeerStore::load_with_note(&config.data_dir)?;
        let index_existed = config.data_dir.join("index.json").exists();
        let (mut index, index_note) = Index::load_with_note(&config.data_dir)?;
        index.remove_tombstones_older_than(now_ms() - TOMBSTONE_TTL_MS);
        // Without the old index, fresh entries would start at one and be
        // dominated by whatever a peer holds for the same paths, letting it
        // replace local edits silently. A floor no counter can have reached
        // makes the first exchange concurrent instead, so a differing file
        // becomes a conflict copy.
        let floor = if index_note.is_some() || (!index_existed && !peers.is_empty()) {
            index.raise_floor((now_ms() / 1000) as u64);
            Some(index.floor())
        } else {
            None
        };

        let listener =
            TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, config.tcp_port)))
                .await
                .with_context(|| format!("listening on TCP port {}", config.tcp_port))?;
        let local_port = listener.local_addr()?.port();

        let trusted_ids: HashSet<String> = peers.ids().into_iter().collect();
        let settings = Settings {
            folder: config.folder.clone(),
            device_name: config.device_name.clone(),
            paused: config.paused,
        };
        let initial = inner::empty_state(&identity, &settings, config.kind, local_port);
        let (state_tx, _) = watch::channel(initial);
        let (dir_tx, _) = broadcast::channel(256);
        let (watch_tx, watch_rx) = mpsc::channel(64);
        let (beacon_tx, beacon_rx) = mpsc::channel(64);
        let (retry_tx, retry_rx) = mpsc::unbounded_channel();

        let engine = Engine {
            shared: Arc::new(Shared {
                identity,
                data_dir: config.data_dir.clone(),
                kind: config.kind,
                poll_watch: config.poll_watch,
                local_port,
                settings: RwLock::new(settings),
                inner: Mutex::new(Inner::new(peers, index)),
                trusted_ids: Arc::new(RwLock::new(trusted_ids)),
                transfers: Arc::new(std::sync::Mutex::new(TransferState::default())),
                state_tx,
                dir_tx,
                state_dirty: Notify::new(),
                index_dirty: Notify::new(),
                watch_tx,
                retry_tx,
                tasks: std::sync::Mutex::new(Vec::new()),
                stopped: AtomicBool::new(false),
                unauthenticated: AtomicUsize::new(0),
                queue_cap: AtomicUsize::new(DEFAULT_QUEUE_CAP),
                hash_delay_ms: std::sync::atomic::AtomicU64::new(0),
                fail_blocks: std::sync::atomic::AtomicU32::new(0),
                tmp_in_flight: std::sync::Mutex::new(HashSet::new()),
            }),
        };

        {
            let mut inner = engine.lock().await;
            for note in [peers_note, index_note].into_iter().flatten() {
                inner.push_error(note);
            }
        }
        if config.beacon_port != 0 {
            let beacon = Beacon::start(
                config.beacon_port,
                engine.advertisement(),
                Vec::new(),
                beacon_tx,
            )?;
            engine.lock().await.beacon = Some(beacon);
        }
        if !config.paused {
            engine.start_folder(floor).await?;
        }
        tasks::spawn_all(&engine, listener, watch_rx, beacon_rx, retry_rx);
        engine.publish_state().await;
        Ok(engine)
    }

    /// The latest published snapshot.
    pub fn state(&self) -> State {
        self.shared.state_tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<State> {
        self.shared.state_tx.subscribe()
    }

    /// Relative directories whose listing changed; `""` is the root.
    pub fn dir_events(&self) -> broadcast::Receiver<String> {
        self.shared.dir_tx.subscribe()
    }

    pub fn local_port(&self) -> u16 {
        self.shared.local_port
    }

    /// Lowers how many downloads a peer may have queued at once. The rest
    /// wait as deferred entries and are offered as the queue drains. For
    /// tests; the default is ten thousand.
    #[doc(hidden)]
    pub fn set_download_queue_cap(&self, cap: usize) {
        self.shared.queue_cap.store(cap.max(1), Ordering::Relaxed);
    }

    /// Makes this device answer the next `count` block requests with
    /// "unavailable", so a test can make a peer's download fail and retry.
    /// For tests.
    #[doc(hidden)]
    pub fn fail_next_blocks_for_tests(&self, count: u32) {
        self.shared.fail_blocks.store(count, Ordering::SeqCst);
    }

    /// Makes the first scan of a folder switch or a resume pause this long
    /// before hashing, so a test can reconnect a peer, or pause the engine,
    /// in the middle of one. For tests.
    #[doc(hidden)]
    pub fn set_scan_hash_delay_for_tests(&self, delay: std::time::Duration) {
        self.shared
            .hash_delay_ms
            .store(delay.as_millis() as u64, Ordering::Relaxed);
    }

    /// Bytes moved over the network so far, in either direction. Local
    /// copies made for a rename never count. For diagnostics and tests.
    pub fn network_bytes(&self) -> u64 {
        self.shared
            .transfers
            .lock()
            .expect("transfers lock")
            .total_bytes()
    }

    pub fn folder(&self) -> PathBuf {
        self.settings().folder
    }

    /// The absolute path of a relative one. `""` is the folder itself;
    /// anything with `..`, a leading slash, backslashes or empty components
    /// is rejected.
    pub fn absolute_path(&self, rel: &str) -> Result<PathBuf> {
        let folder = self.folder();
        if rel.is_empty() {
            return Ok(folder);
        }
        validate_rel(rel)?;
        Ok(folder.join(rel))
    }

    pub async fn list_dir(&self, rel: &str) -> Result<Vec<DirEntry>> {
        local::list_dir(self, rel).await
    }

    /// Copies files (or whole directories) into `into` and indexes them at
    /// once. Returns how many files were copied.
    pub async fn import_files(&self, sources: Vec<PathBuf>, into: &str) -> Result<u32> {
        local::import_files(self, sources, into).await
    }

    pub async fn import_reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
        &self,
        name: &str,
        into: &str,
        reader: R,
    ) -> Result<()> {
        local::import_reader(self, name, into, reader).await
    }

    pub async fn create_folder(&self, rel: &str) -> Result<()> {
        local::create_folder(self, rel).await
    }

    pub async fn delete_entry(&self, rel: &str) -> Result<()> {
        local::delete_entry(self, rel).await
    }

    pub async fn rename_entry(&self, rel: &str, new_name: &str) -> Result<()> {
        local::rename_entry(self, rel, new_name).await
    }

    /// Switches to another folder and indexes it from scratch.
    pub async fn set_folder(&self, path: PathBuf) -> Result<()> {
        local::set_folder(self, path).await
    }

    pub async fn set_device_name(&self, name: String) -> Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() {
            bail!("the device name is empty");
        }
        self.shared
            .settings
            .write()
            .expect("settings lock")
            .device_name = name;
        let inner = self.lock().await;
        if let Some(beacon) = &inner.beacon {
            beacon.update(self.advertisement());
        }
        drop(inner);
        self.publish_state().await;
        Ok(())
    }

    /// Paused: no watching, scanning or syncing. Paired links stay open:
    /// nothing is sent or adopted on them and block requests are answered
    /// as unavailable, and pairing still works so a phone can pair before
    /// it has storage access. Resuming creates the folder, starts the
    /// watcher, runs the first scan, then sends the full index to every
    /// link and applies what the peers announced meanwhile, exactly as on
    /// a new connection. If the resume fails the engine stays paused and
    /// says why in `errors`, so the person can fix the folder and try again.
    pub async fn set_paused(&self, paused: bool) {
        let was = std::mem::replace(
            &mut self.shared.settings.write().expect("settings lock").paused,
            paused,
        );
        if was == paused {
            return;
        }
        if paused {
            self.lock().await.watcher = None;
        } else {
            let folder = self.folder();
            let resumed = async {
                tokio::fs::create_dir_all(&folder)
                    .await
                    .with_context(|| format!("creating {}", folder.display()))?;
                self.start_folder(None).await
            }
            .await;
            match resumed {
                Err(e) => {
                    self.shared.settings.write().expect("settings lock").paused = true;
                    let mut inner = self.lock().await;
                    inner.watcher = None;
                    inner.push_error(format!("cannot resume syncing: {e:#}"));
                }
                Ok(()) if self.settings().paused => {
                    // A pause overtook the resume: it wins, and whatever the
                    // resume set up on its way is taken down again.
                    self.lock().await.watcher = None;
                }
                Ok(()) => self.resume_links().await,
            }
        }
        self.publish_state().await;
    }

    /// After a resume: the full index to every link, then whatever the
    /// peers announced while paused, as if each had just connected.
    async fn resume_links(&self) {
        let replay: Vec<(String, u64, Vec<crate::index::Entry>)> = {
            let mut inner = self.lock().await;
            let conns: Vec<_> = inner.conns.values().map(|l| l.conn.clone()).collect();
            for conn in &conns {
                self.send_full_index(&inner, conn);
            }
            inner
                .conns
                .values_mut()
                .filter(|l| !l.held.is_empty())
                .map(|l| {
                    (
                        l.conn.peer_id.clone(),
                        l.link_id,
                        l.held.drain().map(|(_, e)| e).collect(),
                    )
                })
                .collect()
        };
        for (peer_id, link_id, entries) in replay {
            self.on_remote_entries(&peer_id, link_id, entries).await;
        }
    }

    pub async fn rescan(&self) -> Result<()> {
        self.on_local_batch(vec![String::new()]).await;
        Ok(())
    }

    pub async fn pair_with_nearby(&self, id: &str) -> Result<()> {
        let addr = {
            let inner = self.lock().await;
            inner.nearby.get(id).map(|h| h.addr)
        };
        let Some(addr) = addr else {
            bail!("that device is no longer nearby");
        };
        peer::pair_outgoing(self.clone(), addr, Some(id.to_string())).await
    }

    pub async fn pair_with_address(&self, host: &str, port: u16) -> Result<()> {
        let host = host.trim();
        if host.is_empty() {
            bail!("the address is empty");
        }
        let mut addrs = tokio::net::lookup_host((host, port))
            .await
            .with_context(|| format!("resolving {host}"))?
            .collect::<Vec<_>>();
        addrs.sort_by_key(|a| !a.is_ipv4());
        let Some(addr) = addrs.first().copied() else {
            bail!("{host} did not resolve to an address");
        };
        peer::pair_outgoing(self.clone(), addr, None).await
    }

    /// Accepts or declines an incoming request, or cancels an outgoing one.
    pub async fn respond_to_pairing(&self, id: &str, accept: bool) -> Result<()> {
        let mut inner = self.lock().await;
        let Some(pending) = inner.pending.as_mut() else {
            bail!("no pairing is pending");
        };
        if pending.id != id {
            bail!("no pairing with {id} is pending");
        }
        match pending.direction {
            PairingDirection::Incoming => {
                if let Some(respond) = pending.respond.take() {
                    let _ = respond.send(accept);
                }
            }
            PairingDirection::Outgoing => {
                if accept {
                    bail!("the other device has to accept");
                }
                pending.conn.close();
                inner.pending = None;
            }
        }
        drop(inner);
        self.publish_state().await;
        Ok(())
    }

    pub async fn forget_peer(&self, id: &str) -> Result<()> {
        let mut inner = self.lock().await;
        let existed = inner.peers.remove(id)?;
        self.shared
            .trusted_ids
            .write()
            .expect("trusted lock")
            .remove(id);
        inner.conns.remove(id);
        if inner.pending.as_ref().is_some_and(|p| p.id == id) {
            if let Some(p) = inner.pending.take() {
                p.conn.close();
            }
        }
        drop(inner);
        self.publish_state().await;
        if !existed {
            bail!("{id} is not a paired device");
        }
        Ok(())
    }

    /// Stops every task and connection and writes the index and peers.
    /// Required before dropping the last handle: background tasks hold
    /// their own handles. Returns only once the tasks have stopped, so
    /// the listening port is free when it does.
    pub async fn shutdown(&self) {
        if self.shared.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        let tasks = std::mem::take(&mut *self.shared.tasks.lock().expect("tasks lock"));
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            let _ = task.await;
        }
        let mut inner = self.lock().await;
        inner.watcher = None;
        inner.beacon = None;
        if let Some(p) = inner.pending.take() {
            p.conn.close();
        }
        inner.conns.clear();
        if let Err(e) = inner.index.save(&self.shared.data_dir) {
            tracing::error!("saving the index at shutdown: {e:#}");
        }
        if let Err(e) = inner.peers.save() {
            tracing::error!("saving peers at shutdown: {e:#}");
        }
    }
}
