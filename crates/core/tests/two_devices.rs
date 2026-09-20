//! Two engines in one process on the loopback interface, paired by address.
//! Every wait is bounded; the suite is meant to finish well under a minute.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use owl_core::hash::{blake3_file, blake3_hex};
use owl_core::state::{EntryStatus, PairingDirection};
use owl_core::{Config, DeviceKind, Engine};
use rand::RngCore;
use tempfile::TempDir;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pairing_produces_matching_codes_and_persists_peers() {
    let a = start("Alpha").await;
    let b = start("Beta").await;
    pair(&a, &b).await;

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
    assert!(wait_for_bytes(&b, "note.bin", &bytes, Duration::from_secs(2)).await);
    let elapsed = started.elapsed();
    eprintln!("create-to-visible latency: {} ms", elapsed.as_millis());
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
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

    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut rx = b.engine.subscribe();
    let collector = {
        let seen = seen.clone();
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                let state = rx.borrow().clone();
                let mut seen = seen.lock().unwrap();
                for t in state.transfers.active {
                    seen.push(t.path);
                }
            }
        })
    };

    a.engine.rename_entry("big.bin", "big2.bin").await.unwrap();
    assert!(wait_for_bytes(&b, "big2.bin", &bytes, WAIT).await);
    assert!(wait_until(|| !b.path("big.bin").exists(), WAIT).await);
    assert_eq!(b.files(), vec!["big2.bin"]);
    assert_eq!(mtime_ms(&a.path("big2.bin")), mtime_ms(&b.path("big2.bin")));

    collector.abort();
    let seen = seen.lock().unwrap().clone();
    assert!(
        !seen.iter().any(|p| p == "big2.bin"),
        "big2.bin was transferred over the network: {seen:?}"
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
    let stranger_dir = tempfile::tempdir().unwrap();
    let stranger = owl_core::identity::Identity::load_or_create(stranger_dir.path()).unwrap();
    let hello = owl_core::proto::Control::Hello {
        id: stranger.id.clone(),
        name: "Stranger".into(),
        kind: DeviceKind::Phone,
        version: owl_core::proto::PROTOCOL_VERSION,
    };
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], a.engine.local_port()));
    let (conn, mut rx) = owl_core::conn::dial(&stranger, Arc::new(|_| false), addr, hello)
        .await
        .expect("the handshake itself succeeds so pairing is possible");
    assert_eq!(conn.peer_id, a.id());

    // Sending index data instead of asking to pair gets the connection closed.
    conn.send(owl_core::proto::Frame::Control(
        owl_core::proto::Control::Index {
            entries: Vec::new(),
        },
    ))
    .await
    .unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(frame) = rx.recv().await {
            eprintln!("stranger received {frame:?}");
        }
    })
    .await;
    assert!(closed.is_ok(), "alpha closed the connection");
    let state = a.engine.state();
    assert!(state.peers.is_empty());
    assert!(state.pending_pairing.is_none());

    a.engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forget_peer_disconnects_and_refuses() {
    let a = start("Alpha").await;
    let b = start("Beta").await;
    pair(&a, &b).await;

    a.engine.forget_peer(&b.id()).await.unwrap();
    assert!(a.engine.state().peers.is_empty());
    assert!(wait_until(|| !connected_to(&b.engine, &a.id()), WAIT).await);
    assert!(a.engine.forget_peer(&b.id()).await.is_err());

    // Beta keeps redialling; nothing it sends gets through any more.
    a.write("secret.txt", b"not for beta");
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert!(!b.path("secret.txt").exists());
    assert!(a.engine.state().peers.is_empty());
    assert!(a.engine.state().pending_pairing.is_none());

    a.engine.shutdown().await;
    b.engine.shutdown().await;
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

    let events = a.engine.dir_events();
    drop(events);
    a.engine.shutdown().await;
    b.engine.shutdown().await;
}
