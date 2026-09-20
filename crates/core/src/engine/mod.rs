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
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Remote tombstones to apply again after a short delay.
    pub(crate) retry_tx: mpsc::UnboundedSender<(String, crate::index::Entry)>,
    pub(crate) tasks: std::sync::Mutex<Vec<JoinHandle<()>>>,
    pub(crate) stopped: AtomicBool,
}

impl Engine {
    pub async fn start(config: Config) -> Result<Engine> {
        std::fs::create_dir_all(&config.data_dir)
            .with_context(|| format!("creating {}", config.data_dir.display()))?;
        std::fs::create_dir_all(&config.folder)
            .with_context(|| format!("creating {}", config.folder.display()))?;
        let identity = Identity::load_or_create(&config.data_dir)?;
        let peers = PeerStore::load(&config.data_dir)?;
        let mut index = Index::load(&config.data_dir)?;
        index.remove_tombstones_older_than(now_ms() - TOMBSTONE_TTL_MS);

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
            }),
        };

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
            engine.start_folder().await?;
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

    /// Paused: no watching, scanning, dialling or syncing. Pairing still
    /// works so a phone can pair before it has storage access.
    pub async fn set_paused(&self, paused: bool) {
        let was = std::mem::replace(
            &mut self.shared.settings.write().expect("settings lock").paused,
            paused,
        );
        if was == paused {
            return;
        }
        if paused {
            let mut inner = self.lock().await;
            inner.watcher = None;
            inner.conns.clear();
            inner.queued_paths.clear();
        } else if let Err(e) = self.start_folder().await {
            self.error(format!("resuming: {e:#}")).await;
        }
        self.publish_state().await;
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
        peer::pair_outgoing(self.clone(), addr).await
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
        peer::pair_outgoing(self.clone(), addr).await
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
    /// their own handles.
    pub async fn shutdown(&self) {
        if self.shared.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        let tasks = std::mem::take(&mut *self.shared.tasks.lock().expect("tasks lock"));
        for task in tasks {
            task.abort();
        }
        let mut inner = self.lock().await;
        inner.watcher = None;
        inner.beacon = None;
        if let Some(p) = inner.pending.take() {
            p.conn.close();
        }
        inner.conns.clear();
        inner.queued_paths.clear();
        if let Err(e) = inner.index.save(&self.shared.data_dir) {
            tracing::error!("saving the index at shutdown: {e:#}");
        }
        if let Err(e) = inner.peers.save() {
            tracing::error!("saving peers at shutdown: {e:#}");
        }
    }
}
