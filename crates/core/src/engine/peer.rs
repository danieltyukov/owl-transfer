//! Connection lifecycle: incoming and outgoing connections, the pairing
//! exchange, the per-peer frame loop, block serving and the download worker.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tracing::{debug, info, warn};

use super::inner::{Deferred, Inner, PeerLink, PendingPairing};
use super::Engine;
use crate::clock::now_ms;
use crate::config::DEFAULT_TCP_PORT;
use crate::conn::{self, Connection};
use crate::hash::is_hex_hash;
use crate::index::Entry;
use crate::pairing::{
    commitment, nonce_from_hex, nonce_to_hex, pairing_code, random_nonce, verify_commitment,
};
use crate::peers::PeerRecord;
use crate::proto::{chunk_entries, Control, Frame, BLOCK_UNAVAILABLE};
use crate::state::PairingDirection;
use crate::transfer::{self, Requester, Served};

/// An unpaired connection must ask to pair within this long.
const PAIR_REQUEST_WAIT: Duration = Duration::from_secs(5);
/// How long the person has to accept.
const PAIR_DECISION_WAIT: Duration = Duration::from_secs(60);
const CHALLENGE_WAIT: Duration = Duration::from_secs(10);
/// After a declined, mismatched or timed-out request an address waits
/// this long before it may ask again.
const PAIR_COOLDOWN_MS: i64 = 10_000;
/// Connections that have not proven a pairing, counted across all
/// addresses.
const MAX_UNAUTHENTICATED: usize = 32;
const DOWNLOAD_ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(2);
/// After the quick attempts, the wait between offers grows from this,
/// doubling each time, up to `DEFER_MAX_MS`.
const DEFER_BASE_MS: i64 = 30_000;
const DEFER_MAX_MS: i64 = 10 * 60 * 1000;
/// A full rescan on connect happens at most this often across all peers.
const CONNECT_RESCAN_MS: i64 = 30_000;

/// Counts a connection that has not proven a pairing while it lives.
struct UnauthGuard(Engine);

impl Drop for UnauthGuard {
    fn drop(&mut self) {
        self.0
            .shared()
            .unauthenticated
            .fetch_sub(1, Ordering::SeqCst);
    }
}

/// An accepted TCP connection: paired peers go straight to sync, anyone
/// else has a few seconds to ask to pair.
pub(crate) async fn handle_incoming(engine: Engine, tcp: TcpStream) {
    let counted = engine
        .shared()
        .unauthenticated
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    let guard = UnauthGuard(engine.clone());
    if counted > MAX_UNAUTHENTICATED {
        debug!("too many unauthenticated connections; dropping one");
        return;
    }
    let hello = engine.hello();
    let (conn, rx) =
        match conn::accept(&engine.shared().identity, engine.trusted_fn(), tcp, hello).await {
            Ok(x) => x,
            Err(e) => {
                debug!("incoming connection failed: {e:#}");
                return;
            }
        };
    if conn.trusted {
        drop(guard);
        run_peer(engine, conn, rx).await;
    } else {
        incoming_pairing(engine, conn, rx, guard).await;
    }
}

/// Dials a paired peer at a known address.
pub(crate) async fn dial_peer(engine: Engine, peer_id: String, addr: SocketAddr) {
    let result = conn::dial(
        &engine.shared().identity,
        engine.trusted_fn(),
        addr,
        engine.hello(),
    )
    .await;
    engine.lock().await.dialing.remove(&peer_id);
    match result {
        Ok((conn, rx)) => {
            if conn.peer_id != peer_id || !conn.trusted {
                debug!("{addr} is not {peer_id}; closing");
                conn.close();
                return;
            }
            run_peer(engine, conn, rx).await;
        }
        Err(e) => debug!("dialling {peer_id} at {addr}: {e:#}"),
    }
}

/// Dials an address to pair. `expected_id` is the device the person chose
/// from the nearby list; whoever else answers is refused. Returns once the
/// code is known and shown in the state, or with an error; the rest of the
/// exchange runs on its own.
pub(crate) async fn pair_outgoing(
    engine: Engine,
    addr: SocketAddr,
    expected_id: Option<String>,
) -> Result<()> {
    if engine.lock().await.pending.is_some() {
        bail!("another pairing is in progress");
    }
    let identity = engine.shared().identity.clone();
    let (conn, mut rx) = conn::dial(&identity, engine.trusted_fn(), addr, engine.hello())
        .await
        .with_context(|| format!("connecting to {addr}"))?;
    if expected_id.is_some_and(|id| id != conn.peer_id) {
        conn.close();
        bail!("the device at {addr} is not the one you chose");
    }
    if conn.trusted {
        // Already paired with whoever answered: nothing to do but sync.
        tokio::spawn(run_peer(engine, conn, rx));
        return Ok(());
    }

    let settings = engine.settings();
    let nonce_mine = random_nonce();
    conn.send(Frame::Control(Control::PairRequest {
        id: identity.id.clone(),
        name: settings.device_name,
        kind: engine.shared().kind,
        commit: commitment(&nonce_mine),
    }))
    .await?;
    let nonce_theirs = match timeout(CHALLENGE_WAIT, rx.recv()).await {
        Ok(Some(Frame::Control(Control::PairChallenge { nonce }))) => nonce,
        Ok(Some(Frame::Control(Control::PairReject { reason }))) => {
            conn.close();
            bail!("{} declined: {reason}", conn.peer_name);
        }
        Ok(other) => {
            conn.close();
            bail!("unexpected answer while pairing: {other:?}");
        }
        Err(_) => {
            conn.close();
            bail!("{} did not answer", conn.peer_name);
        }
    };
    let nonce_theirs = nonce_from_hex(&nonce_theirs).context("malformed pairing challenge")?;
    conn.send(Frame::Control(Control::PairReveal {
        nonce: nonce_to_hex(&nonce_mine),
    }))
    .await?;
    let code = pairing_code(&nonce_theirs, &nonce_mine, &conn.peer_id, &identity.id);
    {
        let mut inner = engine.lock().await;
        if inner.pending.is_some() {
            conn.close();
            bail!("another pairing is in progress");
        }
        inner.pending = Some(PendingPairing {
            id: conn.peer_id.clone(),
            name: conn.peer_name.clone(),
            kind: conn.peer_kind,
            code,
            direction: PairingDirection::Outgoing,
            respond: None,
            conn: conn.clone(),
        });
    }
    engine.mark_state();

    tokio::spawn(async move {
        let outcome = timeout(PAIR_DECISION_WAIT, async {
            loop {
                match rx.recv().await {
                    Some(Frame::Control(Control::PairAccept)) => return Ok(()),
                    Some(Frame::Control(Control::PairReject { reason })) => {
                        return Err(Some(reason))
                    }
                    Some(Frame::Control(Control::Ping)) => {
                        let _ = conn.send(Frame::Control(Control::Pong)).await;
                    }
                    Some(_) => {}
                    // Closed without a word: either the person cancelled
                    // here or the other side went away. Not an error to show.
                    None => return Err(None),
                }
            }
        })
        .await;
        clear_pending(&engine, &conn.peer_id).await;
        match outcome {
            Ok(Ok(())) => {
                store_peer(&engine, &conn).await;
                info!("paired with {} ({})", conn.peer_name, conn.peer_id);
                run_peer(engine, conn, rx).await;
            }
            Ok(Err(Some(reason))) => {
                engine
                    .error(format!("pairing with {} failed: {reason}", conn.peer_name))
                    .await;
                conn.close();
            }
            Ok(Err(None)) => {
                debug!("pairing with {} ended without an answer", conn.peer_name);
                conn.close();
            }
            Err(_) => {
                engine
                    .error(format!("pairing with {} timed out", conn.peer_name))
                    .await;
                conn.close();
            }
        }
    });
    Ok(())
}

async fn reject(conn: &Connection, reason: &str) {
    let _ = conn
        .send(Frame::Control(Control::PairReject {
            reason: reason.to_string(),
        }))
        .await;
    conn.close();
}

async fn cool_down(engine: &Engine, ip: IpAddr) {
    engine
        .lock()
        .await
        .pairing_cooldown
        .insert(ip, now_ms() + PAIR_COOLDOWN_MS);
}

/// The acceptor's side of the exchange. `guard` holds one of the
/// unauthenticated slots until the pairing is decided.
async fn incoming_pairing(
    engine: Engine,
    conn: Connection,
    mut rx: mpsc::Receiver<Frame>,
    guard: UnauthGuard,
) {
    let request = timeout(PAIR_REQUEST_WAIT, rx.recv()).await;
    let (id, name, kind, commit) = match request {
        Ok(Some(Frame::Control(Control::PairRequest {
            id,
            name,
            kind,
            commit,
        }))) => (id, name, kind, commit),
        Ok(Some(_)) => {
            // A device we no longer hold a pairing for is talking sync at
            // us. Say so, so it stops trying.
            debug!("unpaired {} sent sync data; refusing", conn.peer_id);
            reject(&conn, "not paired").await;
            return;
        }
        _ => {
            debug!("unpaired {} sent no pairing request; closing", conn.peer_id);
            conn.close();
            return;
        }
    };
    if id != conn.peer_id {
        reject(&conn, "id does not match the certificate").await;
        return;
    }
    let ip = conn.addr.ip();
    let cooling = engine
        .lock()
        .await
        .pairing_cooldown
        .get(&ip)
        .is_some_and(|until| *until > now_ms());
    if cooling {
        reject(&conn, "try again in a moment").await;
        return;
    }

    let identity = engine.shared().identity.clone();
    let nonce_mine = random_nonce();
    if conn
        .send(Frame::Control(Control::PairChallenge {
            nonce: nonce_to_hex(&nonce_mine),
        }))
        .await
        .is_err()
    {
        return;
    }
    let nonce_theirs = match timeout(CHALLENGE_WAIT, rx.recv()).await {
        Ok(Some(Frame::Control(Control::PairReveal { nonce }))) => nonce_from_hex(&nonce),
        _ => None,
    };
    let Some(nonce_theirs) = nonce_theirs.filter(|n| verify_commitment(n, &commit)) else {
        debug!(
            "{} revealed a nonce that does not match its commitment",
            conn.peer_id
        );
        cool_down(&engine, ip).await;
        reject(&conn, "commitment mismatch").await;
        return;
    };
    let code = pairing_code(&nonce_mine, &nonce_theirs, &identity.id, &conn.peer_id);

    let (respond_tx, respond_rx) = oneshot::channel();
    {
        let mut inner = engine.lock().await;
        if inner.pending.is_some() {
            drop(inner);
            reject(&conn, "busy with another pairing").await;
            return;
        }
        inner.pending = Some(PendingPairing {
            id: conn.peer_id.clone(),
            name,
            kind,
            code,
            direction: PairingDirection::Incoming,
            respond: Some(respond_tx),
            conn: conn.clone(),
        });
    }
    engine.mark_state();

    // Wait for the person, answering pings meanwhile; a closed connection
    // ends the wait.
    let decision = tokio::select! {
        answer = timeout(PAIR_DECISION_WAIT, respond_rx) => match answer {
            Ok(Ok(accept)) => Some(accept),
            _ => None,
        },
        _ = pump_until_closed(&conn, &mut rx) => None,
    };
    clear_pending(&engine, &conn.peer_id).await;
    match decision {
        Some(true) => {
            store_peer(&engine, &conn).await;
            // Paired now: the slot is free for the next stranger.
            drop(guard);
            if conn.send(Frame::Control(Control::PairAccept)).await.is_ok() {
                info!("paired with {} ({})", conn.peer_name, conn.peer_id);
                run_peer(engine, conn, rx).await;
            }
        }
        Some(false) => {
            cool_down(&engine, ip).await;
            reject(&conn, "declined").await;
        }
        None => {
            cool_down(&engine, ip).await;
            reject(&conn, "timed out").await;
        }
    }
}

async fn pump_until_closed(conn: &Connection, rx: &mut mpsc::Receiver<Frame>) {
    while let Some(frame) = rx.recv().await {
        if let Frame::Control(Control::Ping) = frame {
            let _ = conn.send(Frame::Control(Control::Pong)).await;
        }
    }
}

async fn clear_pending(engine: &Engine, peer_id: &str) {
    let mut inner = engine.lock().await;
    if inner.pending.as_ref().is_some_and(|p| p.id == peer_id) {
        inner.pending = None;
    }
    drop(inner);
    engine.mark_state();
}

/// The address to redial a peer at. The dialler knows the listening
/// address; the acceptor only sees a source port, so it keeps the source
/// IP with the default port and lets an authenticated dial correct it.
fn redial_address(conn: &Connection) -> String {
    if conn.outbound {
        conn.addr.to_string()
    } else {
        SocketAddr::new(conn.addr.ip(), DEFAULT_TCP_PORT).to_string()
    }
}

async fn store_peer(engine: &Engine, conn: &Connection) {
    let now = now_ms();
    let mut inner = engine.lock().await;
    let last_address = inner
        .peers
        .get(&conn.peer_id)
        .and_then(|p| p.last_address)
        .or_else(|| Some(redial_address(conn)));
    let record = PeerRecord {
        id: conn.peer_id.clone(),
        name: conn.peer_name.clone(),
        kind: conn.peer_kind,
        paired_at_ms: now,
        last_address,
        last_seen_ms: Some(now),
    };
    if let Err(e) = inner.peers.upsert(record) {
        inner.push_error(format!("saving peers: {e:#}"));
    }
    engine
        .shared()
        .trusted_ids
        .write()
        .expect("trusted lock")
        .insert(conn.peer_id.clone());
    drop(inner);
    engine.mark_state();
}

/// The peer told us, on an authenticated link, that it no longer holds our
/// pairing. Drop ours too so we stop dialling it.
async fn forget_because_refused(engine: &Engine, conn: &Connection) {
    let mut inner = engine.lock().await;
    let _ = inner.peers.remove(&conn.peer_id);
    engine
        .shared()
        .trusted_ids
        .write()
        .expect("trusted lock")
        .remove(&conn.peer_id);
    inner.conns.remove(&conn.peer_id);
    inner.push_error(format!(
        "{} no longer trusts this device. Pair again to reconnect.",
        conn.peer_name
    ));
    drop(inner);
    engine.publish_state().await;
}

/// Registers a connection to a paired peer. When two connections to the
/// same peer exist, the one initiated by the device with the smaller id
/// survives, so both sides keep the same one.
async fn register_link(engine: &Engine, conn: &Connection) -> Option<(u64, Arc<Requester>)> {
    let mut inner = engine.lock().await;
    if !inner.peers.contains(&conn.peer_id) || engine.settings().paused || engine.is_stopped() {
        return None;
    }
    if let Some(existing) = inner.conns.get(&conn.peer_id) {
        let keep_outbound = engine.shared().identity.id < conn.peer_id;
        let existing_survives =
            existing.conn.outbound == conn.outbound || existing.conn.outbound == keep_outbound;
        if existing_survives {
            debug!("dropping duplicate connection to {}", conn.peer_id);
            return None;
        }
        debug!("replacing connection to {}", conn.peer_id);
        inner.conns.remove(&conn.peer_id);
    }
    let link_id = inner.next_link_id;
    inner.next_link_id += 1;
    let requester = Requester::new(conn.clone());
    let (queue_tx, queue_rx) = mpsc::unbounded_channel();
    let queue_len = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let worker = tokio::spawn(download_worker(
        engine.clone(),
        requester.clone(),
        link_id,
        queue_rx,
        queue_len.clone(),
    ));
    inner.conns.insert(
        conn.peer_id.clone(),
        PeerLink {
            link_id,
            conn: conn.clone(),
            requester: requester.clone(),
            queue: queue_tx,
            queue_len,
            deferred: Vec::new(),
            pending: HashSet::new(),
            worker,
            transfers: engine.shared().transfers.clone(),
        },
    );
    let now = now_ms();
    inner.peers.set_last_seen(&conn.peer_id, now);
    if conn.outbound {
        let _ = inner
            .peers
            .set_address(&conn.peer_id, conn.addr.to_string());
    }
    Some((link_id, requester))
}

async fn unregister_link(engine: &Engine, peer_id: &str, link_id: u64) {
    let mut inner = engine.lock().await;
    if inner
        .conns
        .get(peer_id)
        .is_some_and(|l| l.link_id == link_id)
    {
        inner.conns.remove(peer_id);
        inner.peers.set_last_seen(peer_id, now_ms());
    }
    drop(inner);
    engine.mark_state();
}

pub(crate) async fn run_peer(engine: Engine, conn: Connection, mut rx: mpsc::Receiver<Frame>) {
    let peer_id = conn.peer_id.clone();
    let Some((link_id, requester)) = register_link(&engine, &conn).await else {
        conn.close();
        return;
    };
    info!("connected to {} ({})", conn.peer_name, peer_id);
    {
        let mut inner = engine.lock().await;
        engine.send_full_index(&inner, &conn);
        // A missed event can never be permanent: every new connection asks
        // for a full rescan, at most once per half minute across all
        // peers so a flapping link cannot keep the scanner busy.
        let now = now_ms();
        if now - inner.last_connect_rescan_ms >= CONNECT_RESCAN_MS {
            inner.last_connect_rescan_ms = now;
            let _ = engine.shared().watch_tx.try_send(vec![String::new()]);
        }
    }
    engine.mark_state();

    while let Some(frame) = rx.recv().await {
        match frame {
            Frame::Control(control) => match control {
                Control::Index { entries } | Control::IndexUpdate { entries } => {
                    engine.on_remote_entries(&peer_id, link_id, entries).await;
                }
                Control::Request {
                    req_id,
                    path,
                    hash,
                    offset,
                    len,
                } => {
                    tokio::spawn(serve(
                        engine.clone(),
                        conn.clone(),
                        link_id,
                        req_id,
                        path,
                        hash,
                        offset,
                        len,
                    ));
                }
                Control::Ping => {
                    let _ = conn.send(Frame::Control(Control::Pong)).await;
                }
                Control::Pong => {}
                Control::PairReject { reason } => {
                    warn!("{} refused this device: {reason}", conn.peer_name);
                    forget_because_refused(&engine, &conn).await;
                    break;
                }
                Control::Error { message } => warn!("{}: {message}", conn.peer_name),
                other => warn!("unexpected {other:?} from {}", conn.peer_name),
            },
            Frame::Block {
                req_id,
                status,
                data,
            } => requester.deliver(req_id, status, data),
        }
    }
    info!("disconnected from {} ({})", conn.peer_name, peer_id);
    conn.close();
    unregister_link(&engine, &peer_id, link_id).await;
}

impl Engine {
    pub(crate) fn send_full_index(&self, inner: &Inner, conn: &Connection) {
        let entries: Vec<Entry> = inner.index.entries().cloned().collect();
        for chunk in chunk_entries(entries) {
            let _ = conn.try_send(Frame::Control(Control::Index { entries: chunk }));
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn serve(
    engine: Engine,
    conn: Connection,
    link_id: u64,
    req_id: u64,
    path: String,
    hash: String,
    offset: u64,
    len: u32,
) {
    // At most a window of requests is served at once; a peer asking for
    // more than it can read gets "unavailable" and will ask again.
    let Some(_permit) = conn.serve_permit() else {
        let _ = conn.try_send(Frame::Block {
            req_id,
            status: BLOCK_UNAVAILABLE,
            data: bytes::Bytes::new(),
        });
        return;
    };
    let failing = engine
        .shared()
        .fail_blocks
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok();
    if failing {
        let _ = conn
            .send(Frame::Block {
                req_id,
                status: BLOCK_UNAVAILABLE,
                data: bytes::Bytes::new(),
            })
            .await;
        return;
    }
    let settings = engine.settings();
    let served = {
        let inner = engine.lock().await;
        if !inner.is_linked(&conn.peer_id, link_id) || !is_hex_hash(&hash) {
            None
        } else {
            inner
                .index
                .get(&path)
                .filter(|e| e.is_live_file())
                .and_then(|e| {
                    e.hash.as_ref().map(|h| Served {
                        hash: h.clone(),
                        size: e.size,
                        mtime_ms: e.mtime_ms,
                    })
                })
        }
    };
    let total = served.as_ref().map(|s| s.size).unwrap_or(0);
    let frame =
        transfer::serve_request(&settings.folder, served, req_id, &path, &hash, offset, len).await;
    let sent = match &frame {
        Frame::Block {
            status: 0, data, ..
        } => data.len() as u64,
        _ => 0,
    };
    if conn.send(frame).await.is_ok() && sent > 0 {
        engine
            .shared()
            .transfers
            .lock()
            .expect("transfers lock")
            .note_upload(&conn.peer_id, &path, sent, total);
        engine.mark_state();
    }
}

/// One file at a time per peer, eight blocks in flight. A failed download
/// is retried a few times quickly, then offered again from the
/// housekeeping tick with a growing delay for as long as the link lives.
async fn download_worker(
    engine: Engine,
    requester: Arc<Requester>,
    link_id: u64,
    mut queue: mpsc::UnboundedReceiver<Entry>,
    queue_len: Arc<std::sync::atomic::AtomicUsize>,
) {
    let mut attempts: HashMap<String, u32> = HashMap::new();
    while let Some(entry) = queue.recv().await {
        queue_len.fetch_sub(1, Ordering::Relaxed);
        match engine.fetch_entry(&requester, link_id, entry.clone()).await {
            Ok(()) => {
                attempts.remove(&entry.path);
            }
            Err(e) => {
                let n = attempts.entry(entry.path.clone()).or_insert(0);
                *n += 1;
                let n = *n;
                debug!(
                    "download of {} from {} failed (attempt {n}): {e:#}",
                    entry.path,
                    requester.conn().peer_id
                );
                if requester.conn().is_closed() {
                    continue;
                }
                if n < DOWNLOAD_ATTEMPTS {
                    let engine = engine.clone();
                    let peer_id = requester.conn().peer_id.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(RETRY_DELAY).await;
                        engine.requeue(&peer_id, link_id, entry).await;
                    });
                } else {
                    let wait = (DEFER_BASE_MS << (n - DOWNLOAD_ATTEMPTS).min(8)).min(DEFER_MAX_MS);
                    if n == DOWNLOAD_ATTEMPTS {
                        engine
                            .error(format!(
                                "could not fetch {}: {e:#}; will keep trying",
                                entry.path
                            ))
                            .await;
                    }
                    let mut inner = engine.lock().await;
                    if let Some(link) = inner.conns.get_mut(&requester.conn().peer_id) {
                        if link.link_id == link_id {
                            link.pending.insert(entry.path.clone());
                            link.deferred.push(Deferred {
                                entry,
                                retry_at_ms: now_ms() + wait,
                            });
                        }
                    }
                }
            }
        }
    }
}

impl Engine {
    /// Puts an entry back on a link's download queue, if the link is
    /// still the current one.
    pub(crate) async fn requeue(&self, peer_id: &str, link_id: u64, entry: Entry) {
        let mut inner = self.lock().await;
        if !inner.is_linked(peer_id, link_id) {
            return;
        }
        self.queue_download(&mut inner, peer_id, entry);
    }

    /// Offers deferred downloads whose time has come, as far as the queue
    /// cap allows; the rest stay deferred and are tried on the next tick.
    /// Called from the housekeeping tick.
    pub(crate) fn requeue_deferred(&self, inner: &mut Inner, now: i64) {
        let cap = self.shared().queue_cap.load(Ordering::Relaxed);
        for link in inner.conns.values_mut() {
            let (due, mut later): (Vec<Deferred>, Vec<Deferred>) =
                link.deferred.drain(..).partition(|d| d.retry_at_ms <= now);
            for d in due {
                if let Err(entry) = link.enqueue(d.entry, cap) {
                    later.push(Deferred {
                        entry,
                        retry_at_ms: now,
                    });
                }
            }
            link.deferred = later;
        }
    }
}
