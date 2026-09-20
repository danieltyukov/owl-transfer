//! The background loops: accepting connections, consuming watcher and
//! beacon events, redialling, keepalives, periodic rescans, saving the
//! index and publishing state.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::debug;

use super::peer::{dial_peer, handle_incoming};
use super::{Engine, TOMBSTONE_TTL_MS};
use crate::beacon::Heard;
use crate::clock::now_ms;
use crate::index::Entry;
use crate::proto::{Control, Frame};

const DIAL_INTERVAL: Duration = Duration::from_secs(5);
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(1);
const PING_EVERY_TICKS: u64 = 15;
const EXPIRY_EVERY_TICKS: u64 = 24 * 3600;
/// Without a beacon, interfaces are listed this often.
const ADDRESS_EVERY_TICKS: u64 = 10;
const SILENCE_LIMIT_MS: i64 = 45_000;
const RESCAN_INTERVAL: Duration = Duration::from_secs(300);
const INDEX_SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
const STATE_COALESCE: Duration = Duration::from_millis(100);
const TOMBSTONE_RETRY_DELAY: Duration = Duration::from_secs(1);

pub(crate) fn spawn_all(
    engine: &Engine,
    listener: TcpListener,
    watch_rx: mpsc::Receiver<Vec<String>>,
    beacon_rx: mpsc::Receiver<Heard>,
    retry_rx: mpsc::UnboundedReceiver<(String, u64, Entry)>,
) {
    let mut tasks = engine.shared().tasks.lock().expect("tasks lock");
    tasks.push(tokio::spawn(listener_loop(engine.clone(), listener)));
    tasks.push(tokio::spawn(watcher_consumer(engine.clone(), watch_rx)));
    tasks.push(tokio::spawn(beacon_consumer(engine.clone(), beacon_rx)));
    tasks.push(tokio::spawn(dial_loop(engine.clone())));
    tasks.push(tokio::spawn(housekeeping_loop(engine.clone())));
    tasks.push(tokio::spawn(rescan_loop(engine.clone())));
    tasks.push(tokio::spawn(index_saver(engine.clone())));
    tasks.push(tokio::spawn(state_publisher(engine.clone())));
    tasks.push(tokio::spawn(tombstone_retry_loop(engine.clone(), retry_rx)));
}

async fn tombstone_retry_loop(
    engine: Engine,
    mut rx: mpsc::UnboundedReceiver<(String, u64, Entry)>,
) {
    while let Some((peer_id, link_id, entry)) = rx.recv().await {
        let engine = engine.clone();
        tokio::spawn(async move {
            tokio::time::sleep(TOMBSTONE_RETRY_DELAY).await;
            engine
                .on_remote_entries(&peer_id, link_id, vec![entry])
                .await;
        });
    }
}

async fn listener_loop(engine: Engine, listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((tcp, _)) => {
                tokio::spawn(handle_incoming(engine.clone(), tcp));
            }
            Err(e) => {
                debug!("accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn watcher_consumer(engine: Engine, mut rx: mpsc::Receiver<Vec<String>>) {
    while let Some(batch) = rx.recv().await {
        engine.on_local_batch(batch).await;
    }
}

async fn beacon_consumer(engine: Engine, mut rx: mpsc::Receiver<Heard>) {
    while let Some(heard) = rx.recv().await {
        let mut inner = engine.lock().await;
        // Beacons are unauthenticated, so they only steer the next dial;
        // the stored address is written after an authenticated connection.
        let paired = inner.peers.contains(&heard.id);
        if paired {
            inner.peers.set_last_seen(&heard.id, heard.at_ms);
        }
        let changed = inner.nearby.get(&heard.id).is_none_or(|old| {
            old.name != heard.name || old.addr != heard.addr || old.kind != heard.kind
        });
        inner.nearby.insert(heard.id.clone(), heard.clone());
        let should_dial =
            paired && !inner.conns.contains_key(&heard.id) && !inner.dialing.contains(&heard.id);
        if should_dial {
            inner.dialing.insert(heard.id.clone());
            tokio::spawn(dial_peer(engine.clone(), heard.id.clone(), heard.addr));
        }
        drop(inner);
        if changed {
            engine.mark_state();
        }
    }
}

async fn dial_loop(engine: Engine) {
    loop {
        {
            let targets: Vec<(String, SocketAddr)> = {
                let mut inner = engine.lock().await;
                let mut targets = Vec::new();
                for peer in inner.peers.list() {
                    if inner.conns.contains_key(&peer.id) || inner.dialing.contains(&peer.id) {
                        continue;
                    }
                    let addr = inner
                        .nearby
                        .get(&peer.id)
                        .map(|h| h.addr)
                        .or_else(|| peer.last_address.as_deref()?.parse().ok());
                    if let Some(addr) = addr {
                        inner.dialing.insert(peer.id.clone());
                        targets.push((peer.id, addr));
                    }
                }
                targets
            };
            for (id, addr) in targets {
                tokio::spawn(dial_peer(engine.clone(), id, addr));
            }
        }
        tokio::time::sleep(DIAL_INTERVAL).await;
    }
}

async fn housekeeping_loop(engine: Engine) {
    let mut tick: u64 = 0;
    loop {
        tokio::time::sleep(HOUSEKEEPING_INTERVAL).await;
        tick += 1;
        let now = now_ms();
        let mut inner = engine.lock().await;
        let mut pruned = inner.prune(now);
        // The beacon sender lists interfaces every two seconds; without
        // one, list them here.
        let fresh = match &inner.beacon {
            Some(beacon) => Some(beacon.addresses()),
            None if tick % ADDRESS_EVERY_TICKS == 0 => Some(crate::beacon::local_ipv4_addresses()),
            None => None,
        };
        if let Some(fresh) = fresh {
            if fresh != inner.addresses {
                inner.addresses = fresh;
                pruned = true;
            }
        }
        if tick % PING_EVERY_TICKS == 0 {
            let silent: Vec<String> = inner
                .conns
                .values()
                .filter(|l| now - l.conn.last_rx_ms() > SILENCE_LIMIT_MS)
                .map(|l| l.conn.peer_id.clone())
                .collect();
            for id in silent {
                debug!("{id} has been silent for too long; dropping");
                inner.conns.remove(&id);
            }
            for link in inner.conns.values() {
                let _ = link.conn.try_send(Frame::Control(Control::Ping));
            }
        }
        if tick % EXPIRY_EVERY_TICKS == 0 {
            inner
                .index
                .remove_tombstones_older_than(now - TOMBSTONE_TTL_MS);
            engine.mark_index();
        }
        engine.requeue_deferred(&mut inner, now);
        drop(inner);
        // A snapshot every few seconds keeps "last seen" and the transfer
        // rate honest; the publisher drops it if nothing changed.
        if pruned || tick % 5 == 0 {
            engine.mark_state();
        }
    }
}

async fn rescan_loop(engine: Engine) {
    loop {
        tokio::time::sleep(RESCAN_INTERVAL).await;
        engine.on_local_batch(vec![String::new()]).await;
        engine.sweep_temp_files().await;
    }
}

async fn index_saver(engine: Engine) {
    loop {
        engine.shared().index_dirty.notified().await;
        tokio::time::sleep(INDEX_SAVE_DEBOUNCE).await;
        let inner = engine.lock().await;
        if let Err(e) = inner.index.save(&engine.shared().data_dir) {
            tracing::error!("saving the index: {e:#}");
        }
    }
}

async fn state_publisher(engine: Engine) {
    loop {
        engine.shared().state_dirty.notified().await;
        engine.publish_state().await;
        tokio::time::sleep(STATE_COALESCE).await;
    }
}
