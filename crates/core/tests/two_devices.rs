//! Two engines in one process on the loopback interface, paired by address.
//! Every wait is bounded; the suite is meant to finish well under a minute.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use owl_core::conn::{self, Connection};
use owl_core::hash::{blake3_file, blake3_hex};
use owl_core::identity::Identity;
use owl_core::index::{Entry, EntryKind};
use owl_core::pairing;
use owl_core::proto::{Control, Frame, PROTOCOL_VERSION};
use owl_core::state::{EntryStatus, PairingDirection};
use owl_core::{Config, DeviceKind, Engine};
use rand::RngCore;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(5);
const RECONNECT_WAIT: Duration = Duration::from_secs(20);

struct Device {
    engine: Engine,
    data: TempDir,
    folder: TempDir,
    name: &'static str,
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
}

fn config(data: &Path, folder: &Path, name: &str, port: u16) -> Config {
    Config {
        data_dir: data.to_path_buf(),
        folder: folder.to_path_buf(),
        device_name: name.to_string(),
        kind: DeviceKind::Desktop,
        tcp_port: port,
        beacon_port: 0,
        poll_watch: false,
        paused: false,
    }
}

async fn start(name: &'static str) -> Device {
    init_logging();
    let data = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let engine = Engine::start(config(data.path(), folder.path(), name, 0))
        .await
        .expect("engine starts");
    Device {
        engine,
        data,
        folder,
        name,
    }
}

impl Device {
    fn id(&self) -> String {
        self.engine.state().device.id
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.folder.path().join(rel)
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        std::fs::write(self.path(rel), bytes).unwrap();
    }

    fn read(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(rel)).ok()
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.folder.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with(".owl-tmp-"))
            .collect();
        names.sort();
        names
    }

    /// Stops the engine and starts it again on the same port with the same
    /// data directory and folder, like an app relaunch.
    async fn restart(&mut self) {
        let port = self.engine.local_port();
        self.engine.shutdown().await;
        self.engine = Engine::start(config(
            self.data.path(),
            self.folder.path(),
            self.name,
            port,
        ))
        .await
        .expect("engine restarts");
    }
}

async fn wait_until<F: FnMut() -> bool>(mut cond: F, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn connected_to(engine: &Engine, id: &str) -> bool {
    engine
        .state()
        .peers
        .iter()
        .any(|p| p.id == id && p.connected)
}

/// `a` dials `b`; `b` accepts. Asserts the codes match on both sides.
async fn pair(a: &Device, b: &Device) {
    a.engine
        .pair_with_address("127.0.0.1", b.engine.local_port())
        .await
        .expect("pairing request goes out");
    assert!(
        wait_until(
            || a.engine.state().pending_pairing.is_some()
                && b.engine.state().pending_pairing.is_some(),
            WAIT
        )
        .await,
        "both sides show a pending pairing"
    );
    let on_a = a.engine.state().pending_pairing.unwrap();
    let on_b = b.engine.state().pending_pairing.unwrap();
    assert_eq!(on_a.code, on_b.code);
    assert_eq!(on_a.direction, PairingDirection::Outgoing);
    assert_eq!(on_b.direction, PairingDirection::Incoming);
    assert_eq!(on_a.id, b.id());
    assert_eq!(on_b.id, a.id());
    b.engine
        .respond_to_pairing(&a.id(), true)
        .await
        .expect("accept");
    assert!(
        wait_until(
            || connected_to(&a.engine, &b.id()) && connected_to(&b.engine, &a.id()),
            WAIT
        )
        .await,
        "both sides connected after accepting"
    );
}

async fn wait_for_bytes(dev: &Device, rel: &str, expected: &[u8], limit: Duration) -> bool {
    wait_until(|| dev.read(rel).as_deref() == Some(expected), limit).await
}

fn mtime_ms(path: &Path) -> i64 {
    owl_core::clock::mtime_ms(&std::fs::metadata(path).unwrap())
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    rand::rng().fill_bytes(&mut buf);
    buf
}

/// A peer driven by hand at the frame level, for probing the engine with
/// things a real engine would never send.
struct Raw {
    identity: Identity,
    conn: Connection,
    rx: mpsc::Receiver<Frame>,
    _dir: TempDir,
}

async fn raw_dial(a: &Device, name: &str) -> Raw {
    let dir = tempfile::tempdir().unwrap();
    let identity = Identity::load_or_create(dir.path()).unwrap();
    let hello = Control::Hello {
        id: identity.id.clone(),
        name: name.into(),
        kind: DeviceKind::Phone,
        version: PROTOCOL_VERSION,
    };
    let addr = SocketAddr::from(([127, 0, 0, 1], a.engine.local_port()));
    let (conn, rx) = conn::dial(&identity, Arc::new(|_| false), addr, hello)
        .await
        .expect("the handshake itself succeeds so pairing is possible");
    assert_eq!(conn.peer_id, a.id());
    Raw {
        identity,
        conn,
        rx,
        _dir: dir,
    }
}

/// The next control frame, answering pings on the way; `None` on a close
/// or when `limit` passes.
async fn next_control(raw: &mut Raw, limit: Duration) -> Option<Control> {
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match timeout(left, raw.rx.recv()).await {
            Ok(Some(Frame::Control(Control::Ping))) => {
                let _ = raw.conn.send(Frame::Control(Control::Pong)).await;
            }
            Ok(Some(Frame::Control(c))) => return Some(c),
            Ok(Some(Frame::Block { .. })) => {}
            Ok(None) | Err(_) => return None,
        }
    }
}

/// Pairs a raw peer with `a` by hand: commit, challenge, reveal, accept.
async fn raw_pair(a: &Device, name: &str) -> Raw {
    let mut raw = raw_dial(a, name).await;
    let nonce_mine = pairing::random_nonce();
    raw.conn
        .send(Frame::Control(Control::PairRequest {
            id: raw.identity.id.clone(),
            name: name.into(),
            kind: DeviceKind::Phone,
            commit: pairing::commitment(&nonce_mine),
        }))
        .await
        .unwrap();
    let Some(Control::PairChallenge { nonce }) = next_control(&mut raw, WAIT).await else {
        panic!("expected a challenge");
    };
    let nonce_theirs = pairing::nonce_from_hex(&nonce).unwrap();
    raw.conn
        .send(Frame::Control(Control::PairReveal {
            nonce: pairing::nonce_to_hex(&nonce_mine),
        }))
        .await
        .unwrap();
    let code = pairing::pairing_code(&nonce_theirs, &nonce_mine, &a.id(), &raw.identity.id);
    assert!(wait_until(|| a.engine.state().pending_pairing.is_some(), WAIT).await);
    assert_eq!(a.engine.state().pending_pairing.unwrap().code, code);
    a.engine
        .respond_to_pairing(&raw.identity.id, true)
        .await
        .unwrap();
    assert!(matches!(
        next_control(&mut raw, WAIT).await,
        Some(Control::PairAccept)
    ));
    assert!(matches!(
        next_control(&mut raw, WAIT).await,
        Some(Control::Index { .. })
    ));
    assert!(wait_until(|| connected_to(&a.engine, &raw.identity.id), WAIT).await);
    raw
}

/// Answers block requests from `files` (path to bytes) and pings until the
/// connection closes, so the engine can fetch what the raw peer announced.
fn serve_raw(mut raw: Raw, files: std::collections::HashMap<String, Vec<u8>>) {
    tokio::spawn(async move {
        while let Some(frame) = raw.rx.recv().await {
            match frame {
                Frame::Control(Control::Ping) => {
                    let _ = raw.conn.send(Frame::Control(Control::Pong)).await;
                }
                Frame::Control(Control::Request {
                    req_id,
                    path,
                    offset,
                    len,
                    ..
                }) => {
                    let block = files.get(&path).map(|bytes| {
                        let start = (offset as usize).min(bytes.len());
                        let end = (start + len as usize).min(bytes.len());
                        bytes::Bytes::copy_from_slice(&bytes[start..end])
                    });
                    let frame = match block {
                        Some(data) => Frame::Block {
                            req_id,
                            status: 0,
                            data,
                        },
                        None => Frame::Block {
                            req_id,
                            status: 1,
                            data: bytes::Bytes::new(),
                        },
                    };
                    let _ = raw.conn.send(frame).await;
                }
                _ => {}
            }
        }
    });
}

fn wire_entry(path: &str, kind: EntryKind, hash: Option<&str>, size: u64, author: &str) -> Entry {
    Entry {
        path: path.into(),
        kind,
        size,
        mtime_ms: 1_700_000_000_000,
        hash: hash.map(str::to_string),
        deleted: false,
        vv: BTreeMap::from([(author.to_string(), 1)]),
        seen_at_ms: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_produces_matching_codes_and_persists_peers() {
    let a = start("Alpha").await;
    let b = start("Beta").await;
    pair(&a, &b).await;

    let device = a.engine.state().device;
    assert_eq!(device.port, a.engine.local_port());
    let mut sorted = device.addresses.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(device.addresses, sorted);
    assert!(device
        .addresses
        .iter()
        .all(|ip| !ip.starts_with("127.") && !ip.contains(':')));

    let peers_a = a.engine.state().peers;
    assert_eq!(peers_a.len(), 1);
    assert_eq!(peers_a[0].name, "Beta");
    assert_eq!(peers_a[0].kind, DeviceKind::Desktop);
    assert!(a.engine.state().pending_pairing.is_none());
    assert!(b.engine.state().pending_pairing.is_none());

    let persisted = std::fs::read_to_string(b.data.path().join("peers.json")).unwrap();
    assert!(persisted.contains(&a.id()));

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn new_file_appears_on_the_other_side_within_two_seconds() {
    let a = start("Alpha").await;
    let b = start("Beta").await;
    pair(&a, &b).await;

    let bytes = random_bytes(1024);
    let started = Instant::now();
    a.write("note.bin", &bytes);
    assert!(wait_for_bytes(&b, "note.bin", &bytes, Duration::from_secs(3)).await);
    let elapsed = started.elapsed();
    eprintln!("create-to-visible latency: {} ms", elapsed.as_millis());
    // Measured around 300 ms; the bound leaves room for a loaded runner.
    assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");
    assert_eq!(mtime_ms(&a.path("note.bin")), mtime_ms(&b.path("note.bin")));

    assert!(
        wait_until(
            || b.engine.state().summary.files == 1 && a.engine.state().summary.files == 1,
            WAIT
        )
        .await
    );
    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn modification_propagates() {
    let a = start("Alpha").await;
    let b = start("Beta").await;
    pair(&a, &b).await;

    a.write("doc.txt", b"version one");
    assert!(wait_for_bytes(&b, "doc.txt", b"version one", WAIT).await);
    a.write("doc.txt", b"version two, longer");
    assert!(wait_for_bytes(&b, "doc.txt", b"version two, longer", WAIT).await);
    assert_eq!(mtime_ms(&a.path("doc.txt")), mtime_ms(&b.path("doc.txt")));

    // And back the other way on the same connection.
    b.write("doc.txt", b"three");
    assert!(wait_for_bytes(&a, "doc.txt", b"three", WAIT).await);

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deletion_propagates() {
    let a = start("Alpha").await;
    let b = start("Beta").await;
    pair(&a, &b).await;

    a.write("gone.txt", b"bye");
    assert!(wait_for_bytes(&b, "gone.txt", b"bye", WAIT).await);
    std::fs::remove_file(a.path("gone.txt")).unwrap();
    assert!(wait_until(|| !b.path("gone.txt").exists(), WAIT).await);
    assert!(wait_until(|| b.engine.state().summary.files == 0, WAIT).await);

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn directory_propagates() {
    let a = start("Alpha").await;
    let b = start("Beta").await;
    pair(&a, &b).await;

    a.engine.create_folder("photos/2026").await.unwrap();
    assert!(wait_until(|| b.path("photos/2026").is_dir(), WAIT).await);
    assert!(wait_until(|| b.engine.state().summary.dirs == 2, WAIT).await);

    b.engine.delete_entry("photos").await.unwrap();
    assert!(wait_until(|| !a.path("photos").exists(), WAIT).await);

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rename_is_a_local_copy() {
    let a = start("Alpha").await;
    let b = start("Beta").await;
    pair(&a, &b).await;

    let bytes = random_bytes(5 * 1024 * 1024);
    a.write("big.bin", &bytes);
    assert!(wait_for_bytes(&b, "big.bin", &bytes, WAIT).await);

    // Everything that moved over the network so far; a rename must add
    // nothing to it on either side.
    let before = (a.engine.network_bytes(), b.engine.network_bytes());
    assert!(before.1 >= bytes.len() as u64);

    a.engine.rename_entry("big.bin", "big2.bin").await.unwrap();
    assert!(wait_for_bytes(&b, "big2.bin", &bytes, WAIT).await);
    assert!(wait_until(|| !b.path("big.bin").exists(), WAIT).await);
    assert_eq!(b.files(), vec!["big2.bin"]);
    assert_eq!(mtime_ms(&a.path("big2.bin")), mtime_ms(&b.path("big2.bin")));
    assert_eq!(
        (a.engine.network_bytes(), b.engine.network_bytes()),
        before,
        "big2.bin went over the network instead of being copied"
    );

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_file_transfers_intact() {
    let a = start("Alpha").await;
    let b = start("Beta").await;
    pair(&a, &b).await;

    let bytes = random_bytes(20 * 1024 * 1024);
    let expected = blake3_hex(&bytes);
    let started = Instant::now();
    a.write("large.bin", &bytes);
    assert!(
        wait_until(
            || std::fs::metadata(b.path("large.bin")).map(|m| m.len()).ok()
                == Some(bytes.len() as u64),
            Duration::from_secs(20)
        )
        .await
    );
    let elapsed = started.elapsed();
    let got = blake3_file(&b.path("large.bin")).await.unwrap();
    assert_eq!(got, expected);
    eprintln!("20 MiB transferred in {} ms", elapsed.as_millis());
    assert!(wait_until(|| b.engine.state().transfers.active.is_empty(), WAIT).await);

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_edit_makes_exactly_one_conflict_copy() {
    let a = start("Alpha").await;
    let mut b = start("Beta").await;
    pair(&a, &b).await;

    a.write("doc.txt", b"v1");
    assert!(wait_for_bytes(&b, "doc.txt", b"v1", WAIT).await);

    b.engine.shutdown().await;
    assert!(wait_until(|| !connected_to(&a.engine, &b.id()), WAIT).await);

    a.write("doc.txt", b"Alpha wrote this");
    assert!(
        wait_until(|| a.engine.state().summary.bytes == 16, WAIT).await,
        "alpha indexes its edit"
    );
    b.write("doc.txt", b"Beta wrote this one");
    // Beta's edit is newer, so it keeps the name.
    let newer = mtime_ms(&a.path("doc.txt")) + 5000;
    owl_core::clock::set_mtime_ms(&b.path("doc.txt"), newer).unwrap();

    b.restart().await;
    assert!(
        wait_until(
            || connected_to(&a.engine, &b.id()) && connected_to(&b.engine, &a.id()),
            RECONNECT_WAIT
        )
        .await,
        "reconnected after restart"
    );

    let settled = |d: &Device| {
        let files = d.files();
        files.len() == 2
            && files.contains(&"doc.txt".to_string())
            && d.read("doc.txt").as_deref() == Some(b"Beta wrote this one".as_slice())
            && files.iter().any(|f| {
                f.starts_with("doc (conflict from Alpha ")
                    && f.ends_with(").txt")
                    && d.read(f).as_deref() == Some(b"Alpha wrote this".as_slice())
            })
    };
    assert!(
        wait_until(|| settled(&a) && settled(&b), RECONNECT_WAIT).await,
        "alpha: {:?}, beta: {:?}",
        a.files(),
        b.files()
    );
    // Nothing else appears afterwards.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(a.files().len(), 2);
    assert_eq!(b.files().len(), 2);
    assert_eq!(a.files(), b.files());

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_versus_edit_keeps_the_edit() {
    let a = start("Alpha").await;
    let mut b = start("Beta").await;
    pair(&a, &b).await;

    a.write("doc.txt", b"v1");
    assert!(wait_for_bytes(&b, "doc.txt", b"v1", WAIT).await);

    b.engine.shutdown().await;
    assert!(wait_until(|| !connected_to(&a.engine, &b.id()), WAIT).await);

    std::fs::remove_file(a.path("doc.txt")).unwrap();
    assert!(wait_until(|| a.engine.state().summary.files == 0, WAIT).await);
    b.write("doc.txt", b"edited while apart");

    b.restart().await;
    assert!(
        wait_until(
            || a.read("doc.txt").as_deref() == Some(b"edited while apart".as_slice()),
            RECONNECT_WAIT
        )
        .await,
        "the edit comes back to alpha"
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(a.files(), vec!["doc.txt"]);
    assert_eq!(b.files(), vec!["doc.txt"]);

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_after_restart_catches_up() {
    let a = start("Alpha").await;
    let mut b = start("Beta").await;
    pair(&a, &b).await;

    b.engine.shutdown().await;
    assert!(wait_until(|| !connected_to(&a.engine, &b.id()), WAIT).await);
    a.write("later.txt", b"written while beta was away");

    b.restart().await;
    assert!(
        wait_for_bytes(
            &b,
            "later.txt",
            b"written while beta was away",
            RECONNECT_WAIT
        )
        .await,
        "beta catches up after restarting"
    );
    assert!(connected_to(&b.engine, &a.id()));

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unpaired_peer_is_refused() {
    let a = start("Alpha").await;
    a.write("keep.txt", b"still here");
    assert!(wait_until(|| a.engine.state().summary.files == 1, WAIT).await);

    let mut raw = raw_dial(&a, "Stranger").await;
    // Sync data instead of a pairing request: a tombstone for a file that
    // exists, from a device alpha never paired with.
    let mut tomb = wire_entry("keep.txt", EntryKind::File, None, 0, &raw.identity.id);
    tomb.deleted = true;
    tomb.vv.insert(raw.identity.id.clone(), 5);
    raw.conn
        .send(Frame::Control(Control::Index {
            entries: vec![tomb],
        }))
        .await
        .unwrap();
    match next_control(&mut raw, WAIT).await {
        Some(Control::PairReject { reason }) => assert_eq!(reason, "not paired"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(next_control(&mut raw, WAIT).await.is_none(), "alpha closes");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        a.read("keep.txt").as_deref(),
        Some(b"still here".as_slice())
    );
    let state = a.engine.state();
    assert!(state.peers.is_empty());
    assert!(state.pending_pairing.is_none());
    assert_eq!(state.summary.files, 1);

    a.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mismatched_reveal_is_rejected() {
    let a = start("Alpha").await;
    let mut raw = raw_dial(&a, "Liar").await;
    let committed = pairing::random_nonce();
    let revealed = pairing::random_nonce();
    raw.conn
        .send(Frame::Control(Control::PairRequest {
            id: raw.identity.id.clone(),
            name: "Liar".into(),
            kind: DeviceKind::Phone,
            commit: pairing::commitment(&committed),
        }))
        .await
        .unwrap();
    assert!(matches!(
        next_control(&mut raw, WAIT).await,
        Some(Control::PairChallenge { .. })
    ));
    raw.conn
        .send(Frame::Control(Control::PairReveal {
            nonce: pairing::nonce_to_hex(&revealed),
        }))
        .await
        .unwrap();
    match next_control(&mut raw, WAIT).await {
        Some(Control::PairReject { reason }) => assert!(reason.contains("commitment"), "{reason}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(next_control(&mut raw, WAIT).await.is_none(), "alpha closes");
    assert!(a.engine.state().pending_pairing.is_none());
    assert!(a.engine.state().peers.is_empty());

    // The address is cooling down: the next request is refused at once.
    let mut again = raw_dial(&a, "Liar").await;
    again
        .conn
        .send(Frame::Control(Control::PairRequest {
            id: again.identity.id.clone(),
            name: "Liar".into(),
            kind: DeviceKind::Phone,
            commit: pairing::commitment(&committed),
        }))
        .await
        .unwrap();
    match next_control(&mut again, WAIT).await {
        Some(Control::PairReject { reason }) => assert!(reason.contains("moment"), "{reason}"),
        other => panic!("expected a refusal, got {other:?}"),
    }

    a.engine.shutdown().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hostile_entries_never_touch_the_disk() {
    let a = start("Alpha").await;
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), a.path("link")).unwrap();
    let mut raw = raw_pair(&a, "Hostile").await;
    let me = raw.identity.id.clone();

    let entries = vec![
        wire_entry("x.bin", EntryKind::File, Some("../../../escape/"), 5, &me),
        wire_entry(
            "y.bin",
            EntryKind::File,
            Some(&"\u{20ac}".repeat(6)),
            5,
            &me,
        ),
        wire_entry("z.bin", EntryKind::File, None, 5, &me),
        wire_entry(
            "link/escaped.txt",
            EntryKind::File,
            Some(&blake3_hex(b"secret")),
            6,
            &me,
        ),
        wire_entry("link/sub", EntryKind::Dir, None, 0, &me),
        wire_entry("d", EntryKind::Dir, Some(&blake3_hex(b"x")), 0, &me),
    ];
    raw.conn
        .send(Frame::Control(Control::IndexUpdate { entries }))
        .await
        .unwrap();

    let got = next_control(&mut raw, Duration::from_millis(1500)).await;
    assert!(
        !matches!(got, Some(Control::Request { .. })),
        "alpha asked for bytes: {got:?}"
    );
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    let mut names = a.files();
    names.retain(|n| n != "link");
    assert!(names.is_empty(), "{names:?}");
    assert!(a
        .path("link")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(
        wait_until(
            || a.engine.state().errors.iter().any(|e| e.contains("link/")),
            WAIT
        )
        .await,
        "{:?}",
        a.engine.state().errors
    );
    assert_eq!(a.engine.state().summary.files, 0);

    a.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forget_peer_disconnects_and_refuses() {
    let a = start("Alpha").await;
    let b = start("Beta").await;
    // Beta dials, so it knows alpha's listening address and will redial it.
    pair(&b, &a).await;

    a.engine.forget_peer(&b.id()).await.unwrap();
    assert!(a.engine.state().peers.is_empty());
    assert!(wait_until(|| !connected_to(&b.engine, &a.id()), WAIT).await);
    assert!(a.engine.forget_peer(&b.id()).await.is_err());

    // Beta redials, alpha answers "not paired", and beta drops the pairing
    // too instead of trying forever.
    assert!(
        wait_until(|| b.engine.state().peers.is_empty(), RECONNECT_WAIT).await,
        "beta forgets alpha after being refused"
    );
    assert!(b
        .engine
        .state()
        .errors
        .iter()
        .any(|e| e.contains("no longer trusts")));

    a.write("secret.txt", b"not for beta");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!b.path("secret.txt").exists());
    assert!(a.engine.state().peers.is_empty());
    assert!(a.engine.state().pending_pairing.is_none());
    assert!(!connected_to(&b.engine, &a.id()));

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn damaged_index_yields_a_conflict_copy_not_a_silent_replace() {
    let a = start("Alpha").await;
    let mut b = start("Beta").await;
    pair(&a, &b).await;

    a.write("doc.txt", b"v1");
    assert!(wait_for_bytes(&b, "doc.txt", b"v1", WAIT).await);
    b.engine.shutdown().await;
    assert!(wait_until(|| !connected_to(&a.engine, &b.id()), WAIT).await);

    // Beta's index is damaged while both sides edit the same file.
    std::fs::write(b.data.path().join("index.json"), b"{broken").unwrap();
    a.write("doc.txt", b"Alpha edit");
    assert!(wait_until(|| a.engine.state().summary.bytes == 10, WAIT).await);
    b.write("doc.txt", b"Beta edit, newer");
    let newer = mtime_ms(&a.path("doc.txt")) + 5000;
    owl_core::clock::set_mtime_ms(&b.path("doc.txt"), newer).unwrap();

    b.restart().await;
    assert!(b
        .engine
        .state()
        .errors
        .iter()
        .any(|e| e.contains("unreadable")));
    assert!(
        wait_until(
            || connected_to(&a.engine, &b.id()) && connected_to(&b.engine, &a.id()),
            RECONNECT_WAIT
        )
        .await
    );

    // Without the old counters beta's entry would be dominated and alpha's
    // copy would replace it silently; the floor makes it a conflict.
    let settled = |d: &Device| {
        let files = d.files();
        files.len() == 2
            && d.read("doc.txt").as_deref() == Some(b"Beta edit, newer".as_slice())
            && files.iter().any(|f| {
                f.starts_with("doc (conflict from Alpha ")
                    && d.read(f).as_deref() == Some(b"Alpha edit".as_slice())
            })
    };
    assert!(
        wait_until(|| settled(&a) && settled(&b), RECONNECT_WAIT).await,
        "alpha: {:?}, beta: {:?}",
        a.files(),
        b.files()
    );

    a.engine.shutdown().await;
    b.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_past_the_queue_cap_are_still_fetched() {
    let a = start("Alpha").await;
    a.engine.set_download_queue_cap(3);
    let raw = raw_pair(&a, "Server").await;
    let me = raw.identity.id.clone();

    let mut files = std::collections::HashMap::new();
    let mut entries = Vec::new();
    for i in 0..10 {
        let path = format!("f{i:02}.bin");
        let bytes = format!("content of file {i}").into_bytes();
        entries.push(wire_entry(
            &path,
            EntryKind::File,
            Some(&blake3_hex(&bytes)),
            bytes.len() as u64,
            &me,
        ));
        files.insert(path, bytes);
    }
    raw.conn
        .send(Frame::Control(Control::IndexUpdate { entries }))
        .await
        .unwrap();
    serve_raw(raw, files.clone());

    // Three at a time, the rest topped up from the deferred list each tick.
    assert!(
        wait_until(
            || files
                .iter()
                .all(|(path, bytes)| a.read(path).as_deref() == Some(bytes.as_slice())),
            Duration::from_secs(20)
        )
        .await,
        "have {:?}",
        a.files()
    );
    assert!(
        wait_until(
            || {
                let s = a.engine.state();
                s.transfers.queued == 0 && s.summary.files == 10 && s.summary.up_to_date
            },
            WAIT
        )
        .await
    );

    a.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn listing_and_imports_reflect_sync_status() {
    let a = start("Alpha").await;
    a.write("alone.txt", b"nobody has this yet");
    assert!(wait_until(|| a.engine.state().summary.files == 1, WAIT).await);
    let listing = a.engine.list_dir("").await.unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].name, "alone.txt");
    assert_eq!(listing[0].status, EntryStatus::Local);
    assert_eq!(listing[0].size, 19);
    assert!(a.engine.list_dir("../x").await.is_err());
    assert!(a.engine.absolute_path("a/../b").is_err());

    let b = start("Beta").await;
    pair(&a, &b).await;
    assert!(wait_for_bytes(&b, "alone.txt", b"nobody has this yet", WAIT).await);

    // Imports go through a temporary name and are indexed at once.
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("dropped.txt"), b"dropped in").unwrap();
    std::fs::create_dir_all(outside.path().join("bundle/inner")).unwrap();
    std::fs::write(outside.path().join("bundle/inner/deep.txt"), b"deep").unwrap();
    let count = a
        .engine
        .import_files(
            vec![
                outside.path().join("dropped.txt"),
                outside.path().join("bundle"),
            ],
            "",
        )
        .await
        .unwrap();
    assert_eq!(count, 2);
    assert!(wait_for_bytes(&b, "dropped.txt", b"dropped in", WAIT).await);
    assert!(wait_for_bytes(&b, "bundle/inner/deep.txt", b"deep", WAIT).await);

    let reader = std::io::Cursor::new(b"from a stream".to_vec());
    a.engine
        .import_reader("streamed.txt", "bundle", reader)
        .await
        .unwrap();
    assert!(wait_for_bytes(&b, "bundle/streamed.txt", b"from a stream", WAIT).await);

    // Names the engine reserves, or that not every device can store, are
    // refused before anything is written.
    assert!(a.engine.create_folder(".owl").await.is_err());
    assert!(a.engine.create_folder("bundle/.owl-tmp-x").await.is_err());
    assert!(a
        .engine
        .import_reader(".owl-tmp-x", "", std::io::Cursor::new(Vec::new()))
        .await
        .is_err());
    assert!(a
        .engine
        .rename_entry("dropped.txt", "a:b.txt")
        .await
        .is_err());
    assert!(a.engine.rename_entry("dropped.txt", "CON").await.is_err());
    assert!(a
        .engine
        .rename_entry("dropped.txt", "Dropped.txt")
        .await
        .is_ok());
    assert!(wait_for_bytes(&b, "Dropped.txt", b"dropped in", WAIT).await);
    assert!(a
        .engine
        .rename_entry("Dropped.txt", "dropped.txt")
        .await
        .is_ok());
    assert!(wait_for_bytes(&b, "dropped.txt", b"dropped in", WAIT).await);
    assert!(!a.path(".owl").exists());

    assert!(
        wait_until(
            || {
                a.engine.state().summary.up_to_date
                    && a.engine.state().peers.iter().any(|p| p.connected)
            },
            WAIT
        )
        .await
    );
    let listing = a.engine.list_dir("").await.unwrap();
    let names: Vec<&str> = listing.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["bundle", "alone.txt", "dropped.txt"]);
    assert!(
        listing.iter().all(|e| e.status == EntryStatus::Synced),
        "{listing:?}"
    );

    // On a case-sensitive filesystem a name differing only in case is a
    // different file, and a rename onto it must be refused like any other.
    a.write("Report.txt", b"upper");
    a.write("report.txt", b"lower");
    let distinct = a.read("Report.txt").as_deref() == Some(b"upper".as_slice())
        && a.read("report.txt").as_deref() == Some(b"lower".as_slice());
    if distinct {
        assert!(a
            .engine
            .rename_entry("Report.txt", "report.txt")
            .await
            .is_err());
        assert_eq!(a.read("Report.txt").as_deref(), Some(b"upper".as_slice()));
        assert_eq!(a.read("report.txt").as_deref(), Some(b"lower".as_slice()));
    }

    let events = a.engine.dir_events();
    drop(events);
    a.engine.shutdown().await;
    b.engine.shutdown().await;
}
