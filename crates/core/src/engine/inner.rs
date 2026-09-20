//! The engine's mutable state and the small helpers every part of it uses.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, MutexGuard};
use tokio::task::JoinHandle;

use super::{Engine, Shared};
use crate::beacon::{Advertisement, Heard, EXPIRY_MS};
use crate::clock::now_ms;
use crate::config::DeviceKind;
use crate::conn::Connection;
use crate::identity::Identity;
use crate::index::{Entry, Index};
use crate::peers::PeerStore;
use crate::proto::{Control, PROTOCOL_VERSION};
use crate::state::{
    DeviceInfo, NearbyInfo, PairingDirection, PairingInfo, PeerInfo, State, SyncSummary,
    TransferSummary,
};
use crate::tls::Trusted;
use crate::transfer::Requester;
use crate::watch::Watcher;

/// A local change younger than this shows as `Waiting`.
pub(crate) const RECENT_MS: i64 = 2000;
const MAX_ERRORS: usize = 5;

#[derive(Clone, Debug)]
pub(crate) struct Settings {
    pub folder: PathBuf,
    pub device_name: String,
    pub paused: bool,
}

/// A download that failed repeatedly, to be offered again later. The
/// worker keeps the attempt count per path, so the wait keeps growing.
pub(crate) struct Deferred {
    pub entry: Entry,
    pub retry_at_ms: i64,
}

/// A live connection to a paired peer with its download queue.
pub(crate) struct PeerLink {
    pub link_id: u64,
    pub conn: Connection,
    pub requester: Arc<Requester>,
    pub queue: mpsc::UnboundedSender<Entry>,
    /// Entries in `queue` not yet taken by the worker.
    pub queue_len: Arc<AtomicUsize>,
    /// Downloads waiting for their turn or for a retry; offered again from
    /// the housekeeping tick.
    pub deferred: Vec<Deferred>,
    /// Every path this link still has to fetch, queued or deferred, for
    /// the `Waiting` status and the queued count. Goes away with the link.
    pub pending: HashSet<String>,
    pub worker: JoinHandle<()>,
}

impl PeerLink {
    /// Queues an entry for download. Past `cap` it is handed back so the
    /// caller can keep it as deferred.
    pub fn enqueue(&self, entry: Entry, cap: usize) -> Result<(), Entry> {
        if self.queue_len.load(std::sync::atomic::Ordering::Relaxed) >= cap {
            return Err(entry);
        }
        match self.queue.send(entry) {
            Ok(()) => {
                self.queue_len
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }
            Err(e) => Err(e.0),
        }
    }
}

impl Drop for PeerLink {
    fn drop(&mut self) {
        self.worker.abort();
        self.requester.fail_all();
        self.conn.close();
    }
}

pub(crate) struct PendingPairing {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub code: String,
    pub direction: PairingDirection,
    /// Incoming only: fires with the user's answer.
    pub respond: Option<oneshot::Sender<bool>>,
    pub conn: Connection,
}

pub(crate) struct Inner {
    pub index: Index,
    pub peers: PeerStore,
    pub conns: HashMap<String, PeerLink>,
    /// Everyone heard on the beacon, paired or not, by id.
    pub nearby: HashMap<String, Heard>,
    pub pending: Option<PendingPairing>,
    pub watcher: Option<Watcher>,
    pub beacon: Option<crate::beacon::Beacon>,
    pub errors: VecDeque<String>,
    /// Paths changed locally recently, for the `Waiting` status.
    pub recent_changes: HashMap<String, i64>,
    pub last_change_ms: Option<i64>,
    /// Peers with a dial in progress.
    pub dialing: HashSet<String>,
    /// Directory tombstones given one more chance to apply.
    pub tombstone_retries: HashSet<String>,
    /// This machine's IPv4 addresses, refreshed by the housekeeping tick.
    pub addresses: Vec<String>,
    /// Addresses that may not ask to pair until the given time.
    pub pairing_cooldown: HashMap<IpAddr, i64>,
    /// Directories the last scans could not read, reported once each.
    pub unreadable: HashSet<String>,
    /// When a full rescan was last triggered by a new connection.
    pub last_connect_rescan_ms: i64,
    pub next_link_id: u64,
}

impl Inner {
    pub fn new(peers: PeerStore, index: Index) -> Inner {
        Inner {
            index,
            peers,
            conns: HashMap::new(),
            nearby: HashMap::new(),
            pending: None,
            watcher: None,
            beacon: None,
            errors: VecDeque::new(),
            recent_changes: HashMap::new(),
            last_change_ms: None,
            dialing: HashSet::new(),
            tombstone_retries: HashSet::new(),
            addresses: crate::beacon::local_ipv4_addresses(),
            pairing_cooldown: HashMap::new(),
            unreadable: HashSet::new(),
            last_connect_rescan_ms: 0,
            next_link_id: 1,
        }
    }

    /// Whether any link still has to fetch `path`.
    pub fn is_pending(&self, path: &str) -> bool {
        self.conns.values().any(|l| l.pending.contains(path))
    }

    /// Downloads waiting across all links.
    pub fn pending_count(&self) -> u32 {
        self.conns.values().map(|l| l.pending.len()).sum::<usize>() as u32
    }

    /// Whether `link_id` is the connection currently registered for the
    /// peer. Frames from an older connection, or from a peer forgotten
    /// since they were read, fail this.
    pub fn is_linked(&self, peer_id: &str, link_id: u64) -> bool {
        self.conns
            .get(peer_id)
            .is_some_and(|l| l.link_id == link_id)
    }

    pub fn push_error(&mut self, message: String) {
        tracing::warn!("{message}");
        self.errors.push_back(message);
        while self.errors.len() > MAX_ERRORS {
            self.errors.pop_front();
        }
    }

    /// Drops expired nearby entries and old recent changes. Returns whether
    /// anything was dropped.
    pub fn prune(&mut self, now: i64) -> bool {
        let before = self.nearby.len() + self.recent_changes.len();
        self.nearby.retain(|_, h| now - h.at_ms <= EXPIRY_MS);
        self.recent_changes.retain(|_, t| now - *t < RECENT_MS);
        self.pairing_cooldown.retain(|_, until| *until > now);
        before != self.nearby.len() + self.recent_changes.len()
    }
}

pub(crate) fn empty_state(
    identity: &Identity,
    settings: &Settings,
    kind: DeviceKind,
    port: u16,
) -> State {
    State {
        device: DeviceInfo {
            id: identity.id.clone(),
            name: settings.device_name.clone(),
            kind,
            port,
            addresses: crate::beacon::local_ipv4_addresses(),
        },
        folder: settings.folder.to_string_lossy().into_owned(),
        paused: settings.paused,
        peers: Vec::new(),
        nearby: Vec::new(),
        pending_pairing: None,
        transfers: TransferSummary::default(),
        summary: SyncSummary::default(),
        errors: Vec::new(),
    }
}

impl Engine {
    pub(crate) fn shared(&self) -> &Shared {
        &self.shared
    }

    pub(crate) async fn lock(&self) -> MutexGuard<'_, Inner> {
        self.shared.inner.lock().await
    }

    pub(crate) fn settings(&self) -> Settings {
        self.shared.settings.read().expect("settings lock").clone()
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.shared
            .stopped
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Asks the publisher for a new snapshot.
    pub(crate) fn mark_state(&self) {
        self.shared.state_dirty.notify_one();
    }

    /// Asks the saver to write the index.
    pub(crate) fn mark_index(&self) {
        self.shared.index_dirty.notify_one();
    }

    pub(crate) fn dir_changed(&self, dir: &str) {
        let _ = self.shared.dir_tx.send(dir.to_string());
    }

    pub(crate) async fn error(&self, message: String) {
        self.lock().await.push_error(message);
        self.mark_state();
    }

    pub(crate) fn trusted_fn(&self) -> Trusted {
        let ids = self.shared.trusted_ids.clone();
        Arc::new(move |fp: &str| ids.read().expect("trusted lock").contains(fp))
    }

    pub(crate) fn hello(&self) -> Control {
        Control::Hello {
            id: self.shared.identity.id.clone(),
            name: self.settings().device_name,
            kind: self.shared.kind,
            version: PROTOCOL_VERSION,
        }
    }

    pub(crate) fn advertisement(&self) -> Advertisement {
        Advertisement {
            id: self.shared.identity.id.clone(),
            name: self.settings().device_name,
            kind: self.shared.kind,
            port: self.shared.local_port,
        }
    }

    pub(crate) fn build_state(&self, inner: &mut Inner) -> State {
        let now = now_ms();
        inner.prune(now);
        let settings = self.settings();
        let transfers = self
            .shared
            .transfers
            .lock()
            .expect("transfers lock")
            .summary(inner.pending_count());

        let peers = inner
            .peers
            .list()
            .into_iter()
            .map(|p| {
                let link = inner.conns.get(&p.id);
                PeerInfo {
                    id: p.id.clone(),
                    name: link.map(|l| l.conn.peer_name.clone()).unwrap_or(p.name),
                    kind: link.map(|l| l.conn.peer_kind).unwrap_or(p.kind),
                    connected: link.is_some(),
                    address: link.map(|l| l.conn.addr.to_string()).or(p.last_address),
                    last_seen_ms: p.last_seen_ms,
                    paired_at_ms: p.paired_at_ms,
                }
            })
            .collect();

        let mut nearby: Vec<NearbyInfo> = inner
            .nearby
            .values()
            .filter(|h| !inner.peers.contains(&h.id))
            .map(|h| NearbyInfo {
                id: h.id.clone(),
                name: h.name.clone(),
                kind: h.kind,
                address: h.addr.to_string(),
            })
            .collect();
        nearby.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));

        let pending_pairing = inner.pending.as_ref().map(|p| PairingInfo {
            id: p.id.clone(),
            name: p.name.clone(),
            kind: p.kind,
            code: p.code.clone(),
            direction: p.direction,
        });

        let (files, dirs, bytes) = inner.index.summary();
        let up_to_date = !settings.paused
            && transfers.active.is_empty()
            && transfers.queued == 0
            && inner.recent_changes.is_empty()
            && inner.pending_count() == 0;

        State {
            device: DeviceInfo {
                id: self.shared.identity.id.clone(),
                name: settings.device_name,
                kind: self.shared.kind,
                port: self.shared.local_port,
                addresses: inner.addresses.clone(),
            },
            folder: settings.folder.to_string_lossy().into_owned(),
            paused: settings.paused,
            peers,
            nearby,
            pending_pairing,
            transfers,
            summary: SyncSummary {
                files,
                dirs,
                bytes,
                last_change_ms: inner.last_change_ms,
                up_to_date,
            },
            errors: inner.errors.iter().cloned().collect(),
        }
    }

    /// Builds a snapshot now and publishes it if anything differs.
    pub(crate) async fn publish_state(&self) {
        let state = {
            let mut inner = self.lock().await;
            self.build_state(&mut inner)
        };
        self.shared.state_tx.send_if_modified(|current| {
            if *current == state {
                false
            } else {
                *current = state;
                true
            }
        });
    }
}
