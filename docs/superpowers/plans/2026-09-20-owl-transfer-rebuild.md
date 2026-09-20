# Owl Transfer rebuild: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Tauri 2 app for Linux, Windows and Android that keeps one folder identical on two devices over the LAN within a second, with a Rust engine and a React interface.

**Architecture:** `crates/core` is the engine (identity, discovery, TLS transport, index, watcher, sync) with a single `Engine` type and a `State` snapshot stream. `app/src-tauri` wraps it in Tauri commands and events. `app/src` is a React UI that talks to a `Backend` interface, implemented once for Tauri and once as an in-memory mock for browser dev and tests.

**Tech Stack:** Rust 1.94 (tokio, rustls+ring, rcgen, notify, blake3, serde), Tauri 2.11 (+ plugins dialog, fs, opener, single-instance), React 19, Vite 7, Vitest, TypeScript 5.7, Kotlin for three Android hooks.

**Spec:** `docs/superpowers/specs/2026-09-20-owl-transfer-rebuild-design.md`

## Global constraints

- No emojis anywhere, no em or en dashes in any text or code comment.
- Commit messages: conventional commits scoped by package (`feat(core):`, `feat(ui):`, `feat(app):`, `docs:`, `ci:`). No AI attribution or session trailers.
- Author: global git identity (`danieltyukov`, `60662998+danieltyukov@users.noreply.github.com`).
- Ports: TCP 52734, UDP beacon 52735. Protocol version 1.
- Android: `minSdk 26`, `targetSdk 35`, package `com.owltransfer.app`.
- Product name "Owl Transfer", binary `owl-transfer`, identifier `com.owltransfer.app`, version `0.1.0` everywhere.
- Every colour token has a definition on bare `:root`.
- Rust edition 2021, `cargo clippy -- -D warnings` clean, `cargo fmt` clean.
- The Flutter tree is removed in Task 0; nothing from it is kept.

## Deviation from the plan format

The tasks below define every shared interface in full code (they are the
contract between parallel tracks) and give each behaviour's tests concretely,
but they do not reproduce every implementation body. The executor of each task
writes the body from the spec, the interface, and the tests.

## Tracks and dependencies

    Track A  crates/core           A1 -> A2 -> A3 -> A4 -> A5 -> A6 -> A7
    Track B  app/src (React)       B1 -> B2 -> B3 -> B4 -> B5      (needs nothing from A; uses the mock backend)
    Track C  assets, docs, CI      C1, C2, C3, C4, C5              (independent)
    Track D  app/src-tauri, Android D1 (scaffold, early) -> D2 (after A7) -> D3 (Android, after D2)
    Track E  end to end            E1 (after D2), E2 (after D3), E3 (release readiness)

---

## Shared contracts

### Rust: `crates/core/src/lib.rs` public surface

```rust
pub use config::{Config, DeviceKind};
pub use engine::Engine;
pub use state::{DirEntry, EntryStatus, NearbyInfo, PairingDirection, PairingInfo, PeerInfo, State, SyncSummary, Transfer, TransferDirection, TransferSummary, DeviceInfo};

// config.rs
#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,       // device.json, peers.json, index.json live here
    pub folder: PathBuf,         // the sync folder; created if missing
    pub device_name: String,
    pub kind: DeviceKind,
    pub tcp_port: u16,           // 52734; 0 = any free port (tests)
    pub beacon_port: u16,        // 52735; 0 = beacon disabled
    pub poll_watch: bool,        // add a 2 s PollWatcher (Android)
    pub paused: bool,            // start paused (Android before permission)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind { Desktop, Phone }

// engine.rs
impl Engine {
    pub async fn start(config: Config) -> anyhow::Result<Engine>;
    pub fn state(&self) -> State;
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<State>;
    pub fn dir_events(&self) -> tokio::sync::broadcast::Receiver<String>; // relative dir whose listing changed ("" = root)
    pub fn local_port(&self) -> u16;
    pub fn folder(&self) -> PathBuf;
    pub fn absolute_path(&self, rel: &str) -> anyhow::Result<PathBuf>;   // rejects "..", absolute, or empty components
    pub async fn list_dir(&self, rel: &str) -> anyhow::Result<Vec<DirEntry>>;
    pub async fn import_files(&self, sources: Vec<PathBuf>, into: &str) -> anyhow::Result<u32>;
    pub async fn import_reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(&self, name: &str, into: &str, reader: R) -> anyhow::Result<()>;
    pub async fn create_folder(&self, rel: &str) -> anyhow::Result<()>;
    pub async fn delete_entry(&self, rel: &str) -> anyhow::Result<()>;
    pub async fn rename_entry(&self, rel: &str, new_name: &str) -> anyhow::Result<()>;
    pub async fn set_folder(&self, path: PathBuf) -> anyhow::Result<()>;   // re-indexes from scratch
    pub async fn set_device_name(&self, name: String) -> anyhow::Result<()>;
    pub async fn set_paused(&self, paused: bool);
    pub async fn rescan(&self) -> anyhow::Result<()>;
    pub async fn pair_with_nearby(&self, id: &str) -> anyhow::Result<()>;
    pub async fn pair_with_address(&self, host: &str, port: u16) -> anyhow::Result<()>;
    pub async fn respond_to_pairing(&self, id: &str, accept: bool) -> anyhow::Result<()>;
    pub async fn forget_peer(&self, id: &str) -> anyhow::Result<()>;
    pub async fn shutdown(&self);
}

// state.rs  (all Serialize + Clone + Debug, serde rename_all = "camelCase")
pub struct State {
    pub device: DeviceInfo,
    pub folder: String,
    pub paused: bool,
    pub peers: Vec<PeerInfo>,
    pub nearby: Vec<NearbyInfo>,
    pub pending_pairing: Option<PairingInfo>,
    pub transfers: TransferSummary,
    pub summary: SyncSummary,
    pub errors: Vec<String>,     // last 5, newest last
}
pub struct DeviceInfo { pub id: String, pub name: String, pub kind: DeviceKind, pub port: u16 }
pub struct PeerInfo { pub id: String, pub name: String, pub kind: DeviceKind, pub connected: bool, pub address: Option<String>, pub last_seen_ms: Option<i64>, pub paired_at_ms: i64 }
pub struct NearbyInfo { pub id: String, pub name: String, pub kind: DeviceKind, pub address: String }
pub struct PairingInfo { pub id: String, pub name: String, pub kind: DeviceKind, pub code: String /* "482 913" */, pub direction: PairingDirection }
pub enum PairingDirection { Incoming, Outgoing }   // serde lowercase
pub struct TransferSummary { pub active: Vec<Transfer>, pub queued: u32, pub bytes_per_sec: u64 }
pub struct Transfer { pub path: String, pub peer_id: String, pub direction: TransferDirection, pub bytes_done: u64, pub bytes_total: u64 }
pub enum TransferDirection { Download, Upload }    // serde lowercase
pub struct SyncSummary { pub files: u64, pub dirs: u64, pub bytes: u64, pub last_change_ms: Option<i64>, pub up_to_date: bool }
pub struct DirEntry { pub name: String, pub path: String, pub is_dir: bool, pub size: u64, pub mtime_ms: i64, pub status: EntryStatus }
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EntryStatus { Synced, Syncing { progress: f32 }, Waiting, Conflict, Local /* no connected peer has it yet */ }
```

Device id = lowercase hex SHA-256 of the certificate DER (64 chars).

### Rust: `crates/core/src/proto.rs` wire format

Frame: `u32` big-endian payload length, then payload. Payload byte 0 is the
frame type: `0` control (the rest is JSON), `1` block.

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    Hello { id: String, name: String, kind: DeviceKind, version: u32 },
    PairRequest { id: String, name: String, kind: DeviceKind },
    PairChallenge { nonce: String /* 32 hex */ },
    PairAccept,
    PairReject { reason: String },
    Index { entries: Vec<Entry> },
    IndexUpdate { entries: Vec<Entry> },
    Request { req_id: u64, path: String, hash: String, offset: u64, len: u32 },
    Ping,
    Pong,
    Error { message: String },
}
// Block frame: [1][req_id: u64 BE][status: u8 (0 ok, 1 unavailable)][bytes...]
```

`Entry` (index.rs), also serialised in `index.json`:

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: String,        // relative, '/' separated, NFC
    pub kind: EntryKind,     // File | Dir  (serde lowercase)
    pub size: u64,
    pub mtime_ms: i64,
    pub hash: Option<String>, // blake3 hex, files only
    pub deleted: bool,
    pub vv: BTreeMap<String, u64>,
    pub seen_at_ms: i64,
}
```

Pairing code: `code = u32::from_be_bytes(hmac_sha256(key = nonce_bytes, msg = fp_a_hex || fp_b_hex)[0..4]) % 1_000_000`, formatted `"{:03} {:03}"`. `fp_a` is the fingerprint of the side that received the PairRequest (the acceptor), `fp_b` the requester.

### TypeScript: `app/src/backend/types.ts`

```ts
export type DeviceKind = 'desktop' | 'phone';
export interface DeviceInfo { id: string; name: string; kind: DeviceKind; port: number }
export interface PeerInfo { id: string; name: string; kind: DeviceKind; connected: boolean; address: string | null; lastSeenMs: number | null; pairedAtMs: number }
export interface NearbyInfo { id: string; name: string; kind: DeviceKind; address: string }
export interface PairingInfo { id: string; name: string; kind: DeviceKind; code: string; direction: 'incoming' | 'outgoing' }
export interface Transfer { path: string; peerId: string; direction: 'download' | 'upload'; bytesDone: number; bytesTotal: number }
export interface TransferSummary { active: Transfer[]; queued: number; bytesPerSec: number }
export interface SyncSummary { files: number; dirs: number; bytes: number; lastChangeMs: number | null; upToDate: boolean }
export interface State {
  device: DeviceInfo; folder: string; paused: boolean; peers: PeerInfo[]; nearby: NearbyInfo[];
  pendingPairing: PairingInfo | null; transfers: TransferSummary; summary: SyncSummary; errors: string[];
}
export type EntryStatus =
  | { kind: 'synced' } | { kind: 'syncing'; progress: number } | { kind: 'waiting' } | { kind: 'conflict' } | { kind: 'local' };
export interface DirEntry { name: string; path: string; isDir: boolean; size: number; mtimeMs: number; status: EntryStatus }
export type Unsubscribe = () => void;
export type Platform = 'desktop' | 'android' | 'browser';
export type Permission = 'granted' | 'denied' | 'not-applicable';
export interface WindowFrame { minimize(): Promise<void>; toggleMaximize(): Promise<void>; close(): Promise<void>; startDrag(): Promise<void> }

export interface Backend {
  readonly platform: Platform;
  readonly version: string;
  readonly window: WindowFrame | null;
  getState(): Promise<State>;
  onState(cb: (s: State) => void): Unsubscribe;
  onDirChanged(cb: (path: string) => void): Unsubscribe;
  onDrop(cb: (paths: string[]) => void): Unsubscribe;
  listDir(path: string): Promise<DirEntry[]>;
  pickAndImport(into: string): Promise<number>;
  importPaths(paths: string[], into: string): Promise<number>;
  createFolder(path: string): Promise<void>;
  deleteEntry(path: string): Promise<void>;
  renameEntry(path: string, newName: string): Promise<void>;
  openEntry(path: string): Promise<void>;
  revealFolder(): Promise<void>;
  pickFolder(): Promise<string | null>;
  setFolder(path: string): Promise<void>;
  setDeviceName(name: string): Promise<void>;
  pairWithNearby(id: string): Promise<void>;
  pairWithAddress(host: string, port: number): Promise<void>;
  respondToPairing(id: string, accept: boolean): Promise<void>;
  forgetPeer(id: string): Promise<void>;
  allFilesPermission(): Promise<Permission>;
  openAllFilesSettings(): Promise<void>;
  openUrl(url: string): Promise<void>;
}
```

Tauri command names are the snake_case of the Backend method names
(`list_dir`, `pick_and_import`, ...). Events: `state` (payload `State`),
`dir-changed` (payload `string`).

---

## Task 0: Clear the ground

**Files:** delete `lib/`, `android/`, `linux/`, `test/`, `pubspec.yaml`, `pubspec.lock`, `analysis_options.yaml`, `.metadata`; replace `.gitignore`; keep `LICENSE`; `README.md` is rewritten in C4.

- [ ] `git rm -r lib android linux test pubspec.yaml pubspec.lock analysis_options.yaml .metadata`
- [ ] Write `.gitignore`: `node_modules/`, `dist/`, `target/`, `app/src-tauri/gen/android/app/build/`, `app/src-tauri/gen/android/.gradle/`, `app/src-tauri/gen/android/build/`, `app/src-tauri/gen/android/app/.cxx/`, `app/src-tauri/gen/android/local.properties`, `*.jks`, `*.keystore`, `keystore.properties`, `.DS_Store`, `*.log`, `.idea/`, `/site/dist/`, `/app/dist/`, `.vite/`, `tsconfig.tsbuildinfo`.
- [ ] Root `Cargo.toml`: `[workspace] members = ["crates/core", "app/src-tauri"] resolver = "2"`, plus `[profile.release] codegen-units = 1, lto = true, opt-level = "s", panic = "abort", strip = true`.
- [ ] Root `package.json`: `{ "name": "owl-transfer", "private": true, "type": "module", "workspaces": ["app", "site"], "scripts": { "build": "npm run build -w app", "test": "vitest run", "typecheck": "tsc -b --pretty false", "dev": "npm run dev -w app" }, "devDependencies": { "typescript": "^5.7.0", "vitest": "^3.2.0", "@types/node": "^22.0.0" } }`; root `tsconfig.json` with references to `app`; `vitest.config.ts` with a project for `app`.
- [ ] `.editorconfig` (2 spaces, LF, utf-8; 4 spaces for `*.rs`, `*.kt`).
- [ ] Commit: `chore: remove the Flutter prototype and lay out the workspace`.

---

## Track A: the engine (`crates/core`)

`crates/core/Cargo.toml` (package `owl-core`, lib): `tokio` (full), `tokio-rustls 0.26`, `rustls 0.23` (default-features off, features `ring`, `std`, `tls12` off, `logging`), `rcgen 0.13` (features `pem`), `rustls-pemfile 2`, `x509-parser` not needed (fingerprint is over the DER we hold), `sha2`, `hmac`, `blake3`, `serde`, `serde_json`, `notify 7`, `notify-debouncer-full 0.4` (or hand-rolled debounce), `anyhow`, `thiserror`, `tracing`, `rand`, `hex`, `socket2`, `unicode-normalization`, `walkdir`, `filetime`, `hostname`, `dirs` (not needed; the app passes dirs), `tempfile` (dev), `tracing-subscriber` (dev). Pin `ring` builds on Android by `rustls = { version = "0.23", default-features = false, features = ["ring", "std", "logging"] }`.

### Task A1: identity, peers store, pairing code

**Files:** `crates/core/src/identity.rs`, `crates/core/src/peers.rs`, `crates/core/src/pairing.rs`, `crates/core/src/lib.rs`, `crates/core/src/config.rs`, `crates/core/src/state.rs`

**Produces:**
```rust
pub struct Identity { pub id: String, pub cert_der: Vec<u8>, pub key_der: Vec<u8> }
impl Identity {
    pub fn load_or_create(data_dir: &Path) -> anyhow::Result<Identity>;   // data_dir/device.json {cert_pem, key_pem}
    pub fn fingerprint(cert_der: &[u8]) -> String;                         // sha256 hex lowercase
}
pub struct PeerRecord { pub id: String, pub name: String, pub kind: DeviceKind, pub paired_at_ms: i64, pub last_address: Option<String> }
pub struct PeerStore { .. }
impl PeerStore {
    pub fn load(data_dir: &Path) -> anyhow::Result<PeerStore>;  // data_dir/peers.json, missing = empty
    pub fn list(&self) -> Vec<PeerRecord>;
    pub fn get(&self, id: &str) -> Option<PeerRecord>;
    pub fn upsert(&mut self, rec: PeerRecord) -> anyhow::Result<()>;      // persists
    pub fn remove(&mut self, id: &str) -> anyhow::Result<bool>;
    pub fn set_address(&mut self, id: &str, addr: String) -> anyhow::Result<()>;
}
pub fn pairing_code(nonce: &[u8; 16], acceptor_fp: &str, requester_fp: &str) -> String; // "482 913"
```

- [ ] Test `identity::tests::create_then_load_is_stable`: `load_or_create` twice in a temp dir yields the same `id`; the id is 64 lowercase hex chars; `device.json` exists.
- [ ] Test `identity::tests::fingerprint_matches_sha256`: `fingerprint(b"abc")` equals the known SHA-256 of "abc".
- [ ] Test `peers::tests::upsert_persists_and_reloads`, `remove_returns_whether_it_existed`.
- [ ] Test `pairing::tests::code_is_six_digits_with_a_space` and `code_differs_when_roles_swap` (swap acceptor/requester, expect a different code with overwhelming probability, assert not equal for a fixed nonce with known vectors: compute once, hardcode the expected string in the test).
- [ ] Implement. `rcgen::generate_simple_self_signed(vec!["owl-transfer".into()])` with `PKCS_ED25519`.
- [ ] `cargo test -p owl-core` green; commit `feat(core): identity, peer store and pairing code`.

### Task A2: index, version vectors, ignore rules, path rules

**Files:** `crates/core/src/index.rs`, `crates/core/src/vv.rs`, `crates/core/src/ignore.rs`, `crates/core/src/paths.rs`

**Produces:**
```rust
// vv.rs
pub type VersionVector = BTreeMap<String, u64>;
pub enum Ordering { Equal, Dominates, Dominated, Concurrent }
pub fn compare(a: &VersionVector, b: &VersionVector) -> Ordering;
pub fn merge(a: &VersionVector, b: &VersionVector) -> VersionVector;   // pointwise max
pub fn bump(vv: &mut VersionVector, device: &str);                     // vv[device] = max over all values + 1
// ignore.rs
pub fn is_ignored(rel_path: &str) -> bool;  // any component: ".owl", starts with ".owl-tmp-", ".DS_Store", "Thumbs.db", "desktop.ini", starts with "~$"; file name ends with ".crdownload" or ".part"
// paths.rs
pub fn normalize_rel(path: &Path, root: &Path) -> Option<String>;      // strip root, '/' separators, NFC; None if outside root
pub fn validate_rel(rel: &str) -> anyhow::Result<()>;                   // no "", no "..", no leading '/', no '\\', no NUL
pub fn conflict_name(name: &str, device_name: &str, when_ms: i64) -> String; // "report (conflict from Pixel 2026-09-20 13-45).pdf"
// index.rs
pub struct Index { .. }   // path -> Entry
impl Index {
    pub fn load(data_dir: &Path) -> anyhow::Result<Index>;      // index.json; missing = empty
    pub fn get(&self, path: &str) -> Option<&Entry>;
    pub fn insert(&mut self, e: Entry);
    pub fn remove_tombstones_older_than(&mut self, ms: i64);
    pub fn entries(&self) -> impl Iterator<Item = &Entry>;
    pub fn live_under(&self, dir: &str) -> Vec<&Entry>;         // direct children of dir ("" = root), not deleted
    pub fn find_by_hash(&self, hash: &str) -> Option<&Entry>;   // a live file with this hash
    pub fn save(&self, data_dir: &Path) -> anyhow::Result<()>;  // atomic: write index.json.tmp then rename
    pub fn summary(&self) -> (u64 /*files*/, u64 /*dirs*/, u64 /*bytes*/);
}
```

- [ ] Tests for `compare` (all four outcomes, including a key missing on one side = 0), `merge`, `bump` (uses max+1, so a device that was behind jumps ahead).
- [ ] Tests for `is_ignored` (each rule, and a normal path returns false), `normalize_rel` (backslashes on Windows via `Path::new` join, NFD to NFC of "é"), `validate_rel` rejects "../x", "/x", "", "a\\b".
- [ ] Test `conflict_name` keeps the extension and handles a name without one.
- [ ] Tests for `Index::save` then `load` round-trip and `live_under("")` excludes tombstones and nested paths.
- [ ] Commit `feat(core): index, version vectors and path rules`.

### Task A3: scanner and watcher

**Files:** `crates/core/src/scan.rs`, `crates/core/src/watch.rs`, `crates/core/src/hash.rs`

**Produces:**
```rust
// hash.rs
pub async fn blake3_file(path: &Path) -> anyhow::Result<String>;  // streaming, spawn_blocking, 1 MiB buffer
// scan.rs
pub struct ScanOutcome { pub changed: Vec<Entry>, pub dirs_touched: BTreeSet<String> }
pub async fn scan_paths(root: &Path, index: &mut Index, device_id: &str, rels: &[String], now_ms: i64) -> anyhow::Result<ScanOutcome>;
   // for each rel: if it is a dir on disk, walk it (walkdir) and compare every file/dir under it; if a file, compare it; if missing, tombstone it and everything under it.
   // compare = size or mtime_ms differs from index (files) or kind differs -> hash (files) -> if hash differs (or new) -> bump vv, insert, push to changed.
   // a file whose size and mtime match the index is not rehashed.
   // a file whose hash matches the index but mtime changed: update mtime only, no bump, not in `changed`.
pub async fn scan_all(root: &Path, index: &mut Index, device_id: &str, now_ms: i64) -> anyhow::Result<ScanOutcome>;  // scan_paths(&[""]) plus tombstones for index entries no longer on disk
// watch.rs
pub struct Watcher { .. }
impl Watcher {
    pub fn start(root: PathBuf, poll: bool, tx: tokio::sync::mpsc::Sender<Vec<String>>) -> anyhow::Result<Watcher>;
    // notify RecommendedWatcher recursive; if poll, also PollWatcher with 2 s; events mapped to relative paths (normalize_rel, drop ignored), debounced 300 ms into one batch, sent on tx. Root deletions send [""].
}
```

- [ ] Test `scan::tests::new_file_is_hashed_and_bumped` (temp dir, write "hello", scan_all, entry has size 5, hash = blake3("hello"), vv = {dev: 1}).
- [ ] Test `scan::tests::unchanged_file_is_not_rehashed` (scan twice; second `changed` is empty).
- [ ] Test `scan::tests::touch_without_content_change_keeps_vv` (set mtime forward with `filetime`; vv unchanged; mtime updated).
- [ ] Test `scan::tests::deleted_file_becomes_tombstone` (delete, scan_all, `deleted == true`, vv bumped).
- [ ] Test `scan::tests::ignored_files_are_skipped` (`.owl-tmp-x` not in index).
- [ ] Test `watch::tests::create_is_reported_within_a_second` (tokio test, temp dir, start watcher, write file, `rx.recv()` with 2 s timeout contains "a.txt" or "").
- [ ] Commit `feat(core): scanner and file watcher`.

### Task A4: framing, TLS, connection

**Files:** `crates/core/src/proto.rs`, `crates/core/src/tls.rs`, `crates/core/src/conn.rs`

**Produces:**
```rust
// proto.rs: Control (above), plus
pub enum Frame { Control(Control), Block { req_id: u64, status: u8, data: bytes::Bytes } }
pub fn encode(frame: &Frame) -> bytes::Bytes;       // length-prefixed
pub struct FrameReader<R>;  impl FrameReader<R: AsyncRead+Unpin> { pub fn new(r: R) -> Self; pub async fn next(&mut self) -> anyhow::Result<Option<Frame>>; }  // max frame 4 MiB, else error
pub const PROTOCOL_VERSION: u32 = 1;
pub const BLOCK_SIZE: u32 = 256 * 1024;
// tls.rs
pub fn server_config(identity: &Identity, trusted: Arc<dyn Fn(&str) -> bool + Send + Sync>) -> anyhow::Result<Arc<rustls::ServerConfig>>;
pub fn client_config(identity: &Identity, trusted: Arc<dyn Fn(&str) -> bool + Send + Sync>) -> anyhow::Result<Arc<rustls::ClientConfig>>;
// The verifiers (ServerCertVerifier / ClientCertVerifier) compute the fingerprint of the presented end-entity cert and call `trusted(fp)`. If false, they still succeed (for pairing) but the fingerprint is recorded; conn.rs decides what is allowed. Client verifier: `dangerous()` custom, `supported_verify_schemes` = ring defaults; require_client_auth on the server with the same custom verifier.
pub fn peer_fingerprint(conn: &tokio_rustls::server::TlsStream<TcpStream>) -> Option<String>;  // and for client::TlsStream
// conn.rs
pub struct Connection { pub peer_id: String, pub addr: SocketAddr, pub outbound: bool, tx: mpsc::Sender<Frame>, .. }
impl Connection {
    pub async fn send(&self, f: Frame) -> anyhow::Result<()>;
}
pub async fn accept(identity, trusted, tcp: TcpStream) -> anyhow::Result<(Connection, mpsc::Receiver<Frame>)>;   // TLS accept, read Hello, send Hello
pub async fn dial(identity, trusted, addr: SocketAddr, hello: Control) -> anyhow::Result<(Connection, mpsc::Receiver<Frame>)>;
```

- [ ] Test `proto::tests::roundtrip_control_and_block` through an in-memory duplex (`tokio::io::duplex`).
- [ ] Test `proto::tests::oversized_frame_is_an_error`.
- [ ] Test `tls::tests::two_identities_handshake_over_localhost` (bind 127.0.0.1:0, accept in a task, dial, both sides report the other's fingerprint correctly).
- [ ] Test `conn::tests::hello_is_exchanged` (peer_id on both Connection values equals the other identity's id).
- [ ] Commit `feat(core): framing, TLS and connections`.

### Task A5: discovery beacon

**Files:** `crates/core/src/beacon.rs`

**Produces:**
```rust
pub struct Beacon { .. }
pub struct Heard { pub id: String, pub name: String, pub kind: DeviceKind, pub addr: SocketAddr /* ip from packet, port from payload */, pub at_ms: i64 }
impl Beacon {
    pub fn start(port: u16, me: Advertisement, tx: mpsc::Sender<Heard>) -> anyhow::Result<Beacon>;  // socket2 UDP, SO_REUSEADDR + SO_BROADCAST, bind 0.0.0.0:port; send every 2 s to 255.255.255.255 and each interface broadcast (via `if-addrs` crate); receive loop parses JSON, drops own id
    pub fn update(&self, me: Advertisement);
}
pub struct Advertisement { pub id: String, pub name: String, pub kind: DeviceKind, pub port: u16 }
```

- [ ] Test `beacon::tests::two_beacons_on_loopback_hear_each_other` on a free port: start two with different ids sending to 127.0.0.1 as well (add a `loopback: bool` field for tests so packets also go to 127.0.0.1:port), assert each hears the other within 5 s.
- [ ] Commit `feat(core): UDP discovery beacon`.

### Task A6: the sync core and the engine

**Files:** `crates/core/src/sync.rs`, `crates/core/src/transfer.rs`, `crates/core/src/engine.rs`, `crates/core/src/state.rs`

Engine internals: one `tokio::sync::Mutex<Inner>` holding the index, peer store, connections map (`peer_id -> Connection`), nearby table, pending pairing, transfer state; a `watch::Sender<State>` rebuilt on change with a 100 ms coalescer; a `broadcast::Sender<String>` for dir events. Tasks: listener (accept loop), watcher consumer, beacon consumer, dial loop (every 5 s: for each paired peer not connected, dial `last_address` or the address heard in `nearby`), keepalive loop (Ping 15 s, drop at 45 s), periodic full rescan (5 min), tombstone expiry (daily).

`sync.rs` is pure logic, tested without IO:
```rust
pub enum Decision { Ignore, Adopt, Conflict { winner_remote: bool } }
pub fn decide(local: Option<&Entry>, remote: &Entry, local_device: &str, remote_device: &str) -> Decision;
pub fn apply_adopt(index: &mut Index, remote: &Entry, now_ms: i64) -> Entry;    // stores remote entry (copies vv), returns it
pub fn resolve_conflict(index: &mut Index, local: &Entry, remote: &Entry, winner_remote: bool, local_device: &str, now_ms: i64) -> (Entry /*winner*/, Option<Entry> /*loser as a new conflict-copy entry, hash unchanged*/);
```
Rules per spec: tombstone vs live file: live wins regardless of vv (Adopt if remote live and local deleted; Ignore if local live and remote deleted, but bump local so the peer adopts the file back); otherwise Dominates/Dominated/Equal; Concurrent with two live files with the same hash: Adopt the merged vv without transfer; different hashes: newer mtime wins, tie: larger device id wins.

`transfer.rs`: per-connection request pipeline. `Downloader::fetch(conn, entry) -> Result<()>`: opens `.owl-tmp-<hash>` in the target dir, issues up to 8 outstanding `Request`s of `BLOCK_SIZE`, writes blocks at offsets, verifies blake3, sets mtime with `filetime`, renames into place, records progress in shared transfer state. On the serving side, `Request` is answered by reading the file at offset; if the file's current blake3 (from the index) does not match the requested hash, answer status 1.

Engine behaviours to implement and test in `crates/core/tests/two_devices.rs` (each test: two engines in one process, temp dirs, `tcp_port: 0`, `beacon_port: 0`; pair via `pair_with_address(127.0.0.1, b.local_port())` then `b.respond_to_pairing(a_id, true)`; helper `wait_until(|| cond, 5s)`):

- [ ] `pairing_produces_matching_codes_and_persists_peers`: both `State.pending_pairing.code` are equal before accept; after accept both `peers` lists contain the other with `connected == true`.
- [ ] `new_file_appears_on_the_other_side_within_two_seconds`: write 1 KiB on A, assert bytes equal on B, `mtime_ms` equal, and elapsed < 2 s.
- [ ] `modification_propagates` and `deletion_propagates` and `directory_propagates` (empty dir).
- [ ] `rename_is_a_local_copy` (rename a 5 MiB file on A; B ends with the new name and same bytes; assert no `Transfer` for it was ever seen in B's state stream, meaning the by-hash local copy was used).
- [ ] `large_file_transfers_intact` (20 MiB random; blake3 equal).
- [ ] `concurrent_edit_makes_exactly_one_conflict_copy`: disconnect (drop B by `shutdown`, restart with same data_dir), edit both, reconnect; assert exactly two files: the winner (newer mtime) under the original name and one `(conflict from ...)` file holding the loser's bytes on both sides.
- [ ] `delete_versus_edit_keeps_the_edit`.
- [ ] `reconnect_after_restart_catches_up` (B stops; A writes; B starts again with same data_dir; file arrives).
- [ ] `unpaired_peer_is_refused` (a third identity dials A with `trusted` false and no pairing; A's state never lists it as connected).
- [ ] `forget_peer_disconnects_and_refuses`.
- [ ] Commit `feat(core): sync engine` (may be several commits: `sync rules`, `transfer`, `engine wiring`).

### Task A7: engine polish and Windows check

- [ ] `import_files` copies via streaming (`tokio::fs::copy` to `.owl-tmp-` then rename) so the watcher never sees a half file; `import_reader` same via the reader.
- [ ] `list_dir` merges disk listing with index status: `Synced` if every connected peer... simpler and honest rule: `Local` if no peer is connected; `Syncing{progress}` if a transfer for the path is active; `Waiting` if the entry's vv is not yet acknowledged... the engine does not track acks, so: `Waiting` if the entry changed in the last 2 s and a peer is connected; `Conflict` if the name matches the conflict pattern; else `Synced`. Document this in `state.rs`.
- [ ] `set_folder`: stop watcher, clear index (delete `index.json`), start watcher on the new path, `scan_all`, send `Index` to every connection.
- [ ] `cargo check -p owl-core --target x86_64-pc-windows-gnu` passes (notify's Windows backend, `filetime`, `socket2`).
- [ ] `cargo clippy -p owl-core --all-targets -- -D warnings` clean.
- [ ] Commit `feat(core): imports, listing status and Windows check`.

---

## Track B: the interface (`app/src`)

Vite + React 19 + TypeScript, `app/package.json` with `@tauri-apps/api ^2.11`, `@tauri-apps/plugin-dialog ^2`, `@tauri-apps/plugin-fs ^2`, `@tauri-apps/plugin-opener ^2`, dev: `@tauri-apps/cli ^2.11`, `@vitejs/plugin-react`, `vitest`, `@testing-library/react`, `@testing-library/user-event`, `@testing-library/jest-dom`, `jsdom`. `vite.config.ts`: `clearScreen: false`, `server: { port: 1420, strictPort: true, host: process.env.TAURI_DEV_HOST || false }`, `envPrefix: ['VITE_', 'TAURI_ENV_*']`, `build: { target: ['es2021', 'chrome105', 'safari13'], minify: !process.env.TAURI_ENV_DEBUG }`.

### Task B1: tokens, base, fonts, marks, backend contract and mock

**Files:** `app/src/tokens.css`, `app/src/base.css`, `app/src/fonts/sans/owl-sans.woff2` + `OFL.txt`, `app/src/fonts/mono/owl-mono.woff2` + `OFL.txt` (from C1; until C1 lands, the `@font-face` rules may reference files that are added by C1; the build must not fail on a missing font, so C1 is merged before B5), `app/src/icons/OwlMark.tsx`, `app/src/icons/glyphs.tsx`, `app/src/backend/types.ts` (the contract above), `app/src/backend/mock.ts`, `app/src/backend/resolve.ts`, `app/src/tokens.test.ts`

Tokens (light on `:root`; dark under `@media (prefers-color-scheme: dark) { :root:not([data-theme='light']) }` and again under `:root[data-theme='dark']`, identical):

```
--bg #f8f7f4 / #111214      --surface #ffffff / #191a1d     --surface-2 #f1efea / #1f2024
--border #e6e3dc / #2a2b30  --border-strong #d3cfc6 / #3a3b41
--text #17181a / #e9e7e2    --muted #66686d / #9a9ba1       --faint #7c7e84 / #7f8087
--accent #a35f14 / #e5a54a  --accent-hover #8a4f0f / #efb765   --accent-tint #f7ecdc / #2b2216
--on-accent #ffffff / #111214
--success #2f7a4a / #6fbf8a --danger #b3372e / #e07068     --info #2b5f9e / #7fb0ea
--hover rgba(23,24,26,.045) / rgba(233,231,226,.06)   --pressed .08 / .1
--r-control 6px  --r-panel 10px   --dur-quick 100ms --dur-base 200ms --dur-slow 320ms  --ease cubic-bezier(.165,.84,.44,1)
--font-sans 'Owl Sans', ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif
--font-mono 'Owl Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, monospace
--fs-micro 11px --fs-mini 12px --fs-small 13px --fs-base 15px --fs-large 17px --fs-h2 19px --fs-h1 24px
--ls-micro .07em --ls-h2 -.025em --ls-h1 -.03em
--row-h 40px (44px under pointer: coarse)  --sidebar-w 240px  --titlebar-h 36px
```

`OwlMark`: viewBox 0 0 32 32; brow `M5,11 L16,6.5 L27,11` stroke 2.2 round; eye rings `circle cx=10.5 cy=17 r=5` and `cx=21.5` stroke 2.2 fill none; pupils r=2.1 filled; beak `M14.4,23.2 L17.6,23.2 L16,26.4 Z`. Export `OWL_FACE_PATH` constants so `scripts/render-icons.mjs` and a test can share the drawing.

`mock.ts`: `createMockBackend(seed?)` returns a `Backend` with an in-memory tree (`Map<string, DirEntry>`), a `State` with one paired connected phone and one nearby desktop, and helpers exported for tests: `mock.emitState(patch)`, `mock.emitDirChanged(path)`, `mock.emitDrop(paths)`. Every mutating method updates the tree and emits `dir-changed`. `pairWithNearby` sets `pendingPairing` outgoing with code "482 913"; `respondToPairing(id, true)` moves the nearby device into peers.

`resolve.ts`: `resolveBackend(): Backend` returns `window.__owlBackend` if a shell set it (the Tauri adapter), else the mock. Tauri adapter is Task D2 (`app/src/backend/tauri.ts`), loaded by `main.tsx` only when `'__TAURI_INTERNALS__' in window`.

- [ ] `tokens.test.ts`: parses `tokens.css`; asserts every `--` name defined in either dark block is defined on `:root`; asserts the two dark blocks are identical; asserts contrast of `--text` on `--bg` and `--on-accent` on `--accent-hover` is at least 4.5 in both themes (implement a small relative-luminance function in the test).
- [ ] `OwlMark.test.tsx`: renders with `label` and without (`aria-hidden`).
- [ ] `mock.test.ts`: `createFolder` then `listDir('')` lists it; `deleteEntry` removes; `respondToPairing` promotes.
- [ ] Commit `feat(ui): tokens, marks and the backend contract`.

### Task B2: shell, title bar, navigation, status strip

**Files:** `app/src/App.tsx`, `app/src/App.css`, `app/src/main.tsx`, `app/src/index.html`, `app/src/components/TitleBar.tsx/.css`, `app/src/components/WindowControls.tsx/.css`, `app/src/components/Sidebar.tsx/.css`, `app/src/components/StatusStrip.tsx/.css`, `app/src/components/Wordmark.tsx/.css`, `app/src/theme.ts`, `app/src/format.ts`, `app/src/state.ts`

`state.ts`: `useBackendState(backend)` hook: subscribes to `onState`, initial `getState()`. `theme.ts`: `applyTheme(theme)` sets `data-theme`, persists in `localStorage['owl-theme']`, `useTheme()`. `format.ts`: `formatBytes(n)` ("1.2 MB", binary-free decimal, one decimal under 10), `formatRelative(ms, now)` ("just now", "2 min ago", "yesterday"), `formatRate(bytesPerSec)`.

App layout: `.shell` (safe-area padding) > optional `TitleBar` (only when `backend.window` is set) > `.app` grid: sidebar | main. Under 900 px: one pane at a time, `data-pane="files|devices|settings"`, a bottom tab bar on phone with three tabs (Files, Devices, Settings) using glyphs. `StatusStrip` at the bottom of the main pane: a dot (green connected / grey none / amber syncing), text "Up to date" | "Syncing 3 files, 12 MB/s" | "No device connected", right side: "2 devices" and last change time.

- [ ] `format.test.ts` covers each function with 4 to 6 cases.
- [ ] `App.test.tsx`: with the mock backend renders the sidebar and Files pane; the status strip reads "Up to date" when `upToDate` and a peer is connected; switching `emitState({ peers: [] })` shows "No device connected".
- [ ] `TitleBar.test.tsx`: renders controls only when a frame is given; clicking close calls `frame.close`.
- [ ] Commit `feat(ui): shell, title bar and status strip`.

### Task B3: the Files pane

**Files:** `app/src/components/Files/Files.tsx/.css`, `FileRow.tsx/.css`, `Breadcrumb.tsx`, `EmptyState.tsx/.css`, `DropZone.tsx`, `EntryMenu.tsx/.css`, `app/src/components/Dialog.tsx/.css` (a small accessible modal used for rename, new folder, confirm delete), `app/src/components/Toast.tsx/.css`

Behaviour: lists `listDir(path)`; re-lists on `dir-changed` for the current path or a parent; folders first then files, natural sort; row: glyph by kind (folder, image, document, archive, audio, video, generic by extension), name, size (mono), modified (relative), badge (synced: small green check; syncing: a 14 px ring with progress; waiting: amber dot; conflict: red "conflict" pill; local: grey "only here"). Click a folder navigates; click a file calls `openEntry`. Right click or the row's overflow button opens `EntryMenu` (Open, Rename, Delete). Long press on touch (500 ms) opens the same menu. Toolbar: breadcrumb, "Add files" (`pickAndImport(path)`), "New folder" (Dialog with a name). Drop: `onDrop` from the backend (desktop paths) and HTML5 drag events for the highlight only. Empty state when the listing is empty at root: "Nothing here yet. Add a file, or drop one in, and it will be on your other device in a moment." with the two buttons; if there are no peers at all, a second card "Pair your other device" linking to Devices. Toasts for errors (`errors` in state; show new ones).

- [ ] `Files.test.tsx`: renders mock entries with folders first; clicking a folder updates the breadcrumb and list; "New folder" dialog creates and the list shows it; rename via menu; delete asks for confirmation then removes; badge text for each status.
- [ ] Commit `feat(ui): files pane`.

### Task B4: the Devices pane and pairing

**Files:** `app/src/components/Devices/Devices.tsx/.css`, `PairingCard.tsx/.css`, `DeviceRow.tsx`, `PairByAddress.tsx`

Behaviour: top: `PairingCard` when `pendingPairing` is set: incoming: "{name} wants to pair" + the code in mono at 32 px + Accept / Decline; outgoing: "Pairing with {name}" + code + "Confirm on the other device" + Cancel (`respondToPairing(id, false)`). Sections: "Paired" (rows: kind glyph, name, "Connected" green or "Last seen 3 min ago", address in mono faint, Forget with confirm), "Nearby" (rows with Pair button; empty: "No other device found on this network. Make sure both are on the same Wi-Fi, or pair by address."), "Pair by address" (host input, port input default 52734, Connect). This device's own name, id (first 8 chars) and port shown at the bottom as a small card ("This device"), so the other side can type it.

- [ ] `Devices.test.tsx`: nearby Pair click sets an outgoing card with "482 913"; incoming card Accept calls `respondToPairing(id, true)`; Forget confirms and removes; pair by address calls `pairWithAddress('10.0.2.2', 52734)`.
- [ ] Commit `feat(ui): devices pane and pairing`.

### Task B5: Settings, Android permission card, first-run hint, polish

**Files:** `app/src/components/Settings/Settings.tsx/.css`, `app/src/components/PermissionCard.tsx`

Settings: cards: Device (name input, saved on blur/Enter), Folder (path in mono, Change via `pickFolder` then `setFolder`, Open on desktop via `revealFolder`), Appearance (three segmented buttons: System, Light, Dark), Storage access (Android only: state from `allFilesPermission()`, button `openAllFilesSettings()`; re-check on window focus), About (version, "Owl Transfer is open source" link to the repository via `openUrl`).

- [ ] `Settings.test.tsx`: name change calls `setDeviceName`; theme buttons set `data-theme`; permission card appears only when `platform === 'android'`.
- [ ] Phone layout check in jsdom is not meaningful; instead a Playwright smoke (`app/e2e/smoke.spec.ts`) against `vite preview` with the mock backend at 390x844 and 1280x800: screenshot both, assert the tab bar is visible only on the narrow one. Config in `app/playwright.config.ts`.
- [ ] `npm run typecheck && npm test` green at root; commit `feat(ui): settings and phone layout`.

---

## Track C: assets, docs, site, CI

### Task C1: fonts

**Files:** `scripts/fetch-fonts.sh`, `scripts/rename-font.py` (copy from to-hoot, adjust), output `app/src/fonts/sans/owl-sans.woff2`, `app/src/fonts/mono/owl-mono.woff2`, each with `OFL.txt`.

- [ ] Instrument Sans (wdth=100, wght 400:700) as "Owl Sans"; JetBrains Mono (wght 400:700) as "Owl Mono". Same unicode range and features as to-hoot's script.
- [ ] Verify with `python3 -c "from fontTools.ttLib import TTFont; print(TTFont('app/src/fonts/sans/owl-sans.woff2')['name'].getDebugName(1))"` prints "Owl Sans".
- [ ] Commit `feat(ui): bundle Owl Sans and Owl Mono`.

### Task C2: icons

**Files:** `scripts/render-icons.mjs`, `app/icon-source.svg`, `app/src-tauri/icons/*` (via `npx tauri icon`), `app/public/favicon.svg`, Android launcher assets under `app/src-tauri/gen/android/app/src/main/res/` (written once D1 has generated the project; the script writes them if the directory exists and says so otherwise).

- [ ] The face path constants match `OwlMark.tsx` (a test in `app/src/icons/marks.test.ts` reads `app/icon-source.svg` and asserts the path strings are present).
- [ ] Amber `#e5a54a` ground, ink `#111214` face, rounded square rx 225 for desktop, full bleed for the Android adaptive icon; legacy mipmaps at the five densities.
- [ ] Commit `feat(app): icons`.

### Task C3: the site

**Files:** `site/index.html`, `site/style.css`, `site/theme.js`, `site/vite.config.ts`, `site/package.json`, `docs/img/desktop-light.webp`, `docs/img/desktop-dark.webp`, `docs/img/phone.webp`, `docs/img/phone-dark.webp` (screenshots taken in E1/E2; the site is written first with the four `<img>` slots and alt text, and the files are added when captured).

Content: the mark and wordmark; lede "A folder that is the same on your computer and your phone. Drop a file in on one and it is on the other within a second, over your own Wi-Fi, with no account and no server."; three download buttons (`releases/latest/download/owl-transfer.apk`, `owl-transfer_x86_64.AppImage`, `OwlTransfer_x64-setup.exe`); a note on the deb and msi; the screenshot figure; three short sections: How it works (LAN, TLS, both keep a copy), Pairing (the six-digit code), What it costs (nothing; no service); footer with the repository link and MIT. Imports `../app/src/tokens.css` like to-hoot's site does.

- [ ] `npm run build -w site` succeeds. Commit `docs: project site`.

### Task C4: README, CONTRIBUTING, SECURITY, ARCHITECTURE, issue templates

**Files:** `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `docs/ARCHITECTURE.md`, `.github/ISSUE_TEMPLATE/bug.yml`, `feature.yml`, `config.yml`, `.github/PULL_REQUEST_TEMPLATE.md`

README sections, in this order: title + picture; What it is (four paragraphs: the folder, how sync works in two sentences, pairing, what it is not); Install (Android, Linux, Windows, same shape as to-hoot's); First run (pair the two devices, three steps); What it costs (a short table: nothing); Build from source; Repository layout; Documentation; Licence. Plain prose, no marketing tone, no emojis.

ARCHITECTURE: the engine's model as in the spec, written for someone auditing it: identity, discovery, pairing, connections, index and version vectors, change detection, sync rules with a worked example of a conflict, transfers, what is stored where on each platform, and the threat model (LAN attacker cannot read or inject: TLS with pinned fingerprints; first pairing is protected by the code comparison; a compromised paired device can read and write the folder, which is the point of pairing).

SECURITY: Reporting (GitHub private vulnerability reporting), Where the keys live, The pairing code, Android permissions, Releases (built by Actions from the tag, unsigned installers), Scope.

- [ ] Commit `docs: README, contributing, security and architecture`.

### Task C5: workflows

**Files:** `.github/workflows/ci.yml`, `release.yml`, `pages.yml`

ci.yml jobs: `rust` (ubuntu-latest: `cargo fmt --check`, `cargo clippy -p owl-core --all-targets -- -D warnings`, `cargo test -p owl-core`, `rustup target add x86_64-pc-windows-gnu` + `cargo check -p owl-core --target x86_64-pc-windows-gnu`), `web` (node 22: `npm ci`, `npm run typecheck`, `npm test`, `npm run build -w app`, Playwright smoke), `site` (build).

release.yml on `v*`: `desktop` (ubuntu-22.04, WebKitGTK deps, `npm ci`, `npx tauri build --bundles deb,appimage` in `app`), `windows` (windows-latest, `npx tauri build --bundles nsis,msi`), `android` (ubuntu-latest, setup-java 21, NDK via `android-actions/setup-android` + `sdkmanager "ndk;27.1.12297006"`, rust targets `aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android`, `npx tauri android build --apk --target aarch64 --target x86_64`; signing from the four secrets as in to-hoot with an unsigned fallback via `apksigner`/`zipalign` skipped), `release` (gh release create with the five assets under stable names: `owl-transfer.apk`, `owl-transfer_amd64.deb`, `owl-transfer_x86_64.AppImage`, `OwlTransfer_x64-setup.exe`, `OwlTransfer_x64.msi`).

pages.yml: builds `site/` on push to `master` (this repository's default branch) with the same paths filter idea.

- [ ] `actionlint` if available, else a careful read. Commit `ci: build, release and pages workflows`.

---

## Track D: the Tauri app

### Task D1: scaffold (can start immediately)

**Files:** `app/src-tauri/Cargo.toml`, `app/src-tauri/build.rs`, `app/src-tauri/tauri.conf.json`, `app/src-tauri/tauri.windows.conf.json`, `app/src-tauri/capabilities/default.json`, `app/src-tauri/src/lib.rs`, `app/src-tauri/src/main.rs`, `app/src-tauri/owl-transfer.desktop`, `app/src-tauri/gen/android/**` (from `npx tauri android init`), Android edits below.

`Cargo.toml`: package `owl-transfer`, lib `owl_transfer_lib` with `crate-type = ["lib", "cdylib", "staticlib"]`, deps `tauri = { version = "2", features = [] }`, `tauri-plugin-dialog`, `tauri-plugin-fs`, `tauri-plugin-opener`, `tauri-plugin-single-instance` (desktop only, `[target.'cfg(not(any(target_os = "android", target_os = "ios")))'.dependencies]`), `owl-core = { path = "../../crates/core" }`, `tokio`, `serde`, `serde_json`, `anyhow`, `tracing`, `tracing-subscriber`; Android: `tracing-android` or `android_logger` for logcat.

`tauri.conf.json`: `productName: "Owl Transfer"`, `version: "0.1.0"`, `identifier: "com.owltransfer.app"`, `mainBinaryName: "owl-transfer"`, `build: { beforeDevCommand: "npm run dev", beforeBuildCommand: "npm run build", devUrl: "http://localhost:1420", frontendDist: "../dist" }`, window `label main, title "Owl Transfer", 1100x720, min 720x480, decorations false, center`, CSP `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self' data:; img-src 'self' data: asset: http://asset.localhost; connect-src 'self' ipc: http://ipc.localhost`, bundle targets `deb, appimage`, icons, `linux.deb.desktopTemplate`. `tauri.windows.conf.json`: `productName: "OwlTransfer"`, bundle targets `nsis, msi`, nsis `installMode: currentUser`, webview install mode `downloadBootstrapper`.

`capabilities/default.json`: `core:default`, `core:window:allow-start-dragging`, `core:window:allow-minimize`, `core:window:allow-toggle-maximize`, `core:window:allow-close`, `dialog:default`, `fs:default`, `opener:default`, plus `fs:allow-read-file` scoped to `$HOME/**` on desktop and `content://**` handled by the plugin's Android support.

Android (in `gen/android/app/src/main`): `AndroidManifest.xml` adds `INTERNET`, `ACCESS_NETWORK_STATE`, `ACCESS_WIFI_STATE`, `CHANGE_WIFI_MULTICAST_STATE`, `MANAGE_EXTERNAL_STORAGE`, `READ_EXTERNAL_STORAGE` (maxSdk 32), `WRITE_EXTERNAL_STORAGE` (maxSdk 29), `android:requestLegacyExternalStorage="true"`, `usesCleartextTraffic` not needed (TLS), and the `ACTION_SEND` / `ACTION_SEND_MULTIPLE` intent filters (`*/*`) on the main activity; `MainActivity.kt` acquires the `MulticastLock` in `onCreate`, handles the share intent in `onCreate` and `onNewIntent` by copying every `EXTRA_STREAM` URI into `<folder>/<display name>` (folder read from `filesDir/settings.json` field `folder`, default `/storage/emulated/0/OwlTransfer`; copy to a `.owl-tmp-` name then rename), and exposes two JNI-free helpers via a tiny Tauri mobile plugin `app/src-tauri/src/android.rs` + `gen/android/.../OwlPlugin.kt` with commands `allFilesPermission` and `openAllFilesSettings` (the plugin is registered with `tauri::plugin::Builder::new("owl").setup(|app, api| api.register_android_plugin("com.owltransfer.app", "OwlPlugin"))`).

Desktop `lib.rs::run()`: `tauri::Builder::default()` with the plugins, single instance focusing the main window, `setup` starting the engine on the app's async runtime (`tauri::async_runtime::spawn`) with `data_dir = app.path().app_data_dir()`, folder from `settings.json` (`{ folder, device_name }` in the data dir, default `home/OwlTransfer` or `/storage/emulated/0/OwlTransfer`), `kind` by `cfg!(target_os = "android")`, `poll_watch` on Android, `paused` on Android when the permission is missing; a task forwards `subscribe()` changes to `app.emit("state", ..)` and `dir_events()` to `app.emit("dir-changed", ..)`. The engine is stored in `app.manage(EngineHandle(Arc<OnceCell<Engine>>))`.

- [ ] `npx tauri dev` opens a window showing the mock-backed UI (Tauri adapter lands in D2). `npx tauri android init` generates `gen/android`; `npx tauri android build --debug --target x86_64` produces an APK. Commit `feat(app): tauri scaffold for desktop and android`.

### Task D2: commands and the Tauri adapter (after A7 and B5)

**Files:** `app/src-tauri/src/commands.rs`, `app/src-tauri/src/settings.rs`, `app/src/backend/tauri.ts`, `app/src/main.tsx`

Commands as listed in the contract; each takes `State<'_, EngineHandle>`, awaits the engine, maps `anyhow::Error` to `String`. `pick_and_import(into)` is implemented on the JS side: `open({ multiple: true })` from the dialog plugin, then `import_paths(paths, into)`; on Android the dialog returns `content://` paths and the command `import_files` accepts `Vec<tauri_plugin_fs::FilePath>`, opening each with `app.fs().open(path, OpenOptions::new().read(true))` and calling `engine.import_reader(name, into, tokio::fs::File::from_std(file))`; the display name for a content URI comes from the dialog result's file name when available, else `shared-<timestamp>`. `pick_folder` uses the dialog plugin's `open({ directory: true })` (desktop only; Android returns null and the folder is fixed). `reveal_folder` uses `tauri_plugin_opener::reveal_item_in_dir`. `open_entry` uses `open_path`. `all_files_permission` / `open_all_files_settings` call the `owl` plugin on Android and return `not-applicable` elsewhere.

`tauri.ts`: implements `Backend` with `invoke` and `listen`, `window` via `getCurrentWindow()` on desktop (null on Android), `onDrop` via `getCurrentWebview().onDragDropEvent` filtering `type === 'drop'`, `platform` from `import.meta.env.TAURI_ENV_PLATFORM` (`'android'` when it is `android`, else `'desktop'`), `version` from `getVersion()`.

- [ ] Manual check: `npx tauri dev`, the Files pane lists `~/OwlTransfer`, adding a file via the dialog shows it, dropping a file shows it. Commit `feat(app): commands and the tauri backend`.

### Task D3: Android build and hooks (after D2)

- [ ] `npx tauri android build --debug --target x86_64` installs on the running emulator via `adb install -r`; the app opens, shows the permission card, `adb shell appops set com.owltransfer.app MANAGE_EXTERNAL_STORAGE allow` (or tapping through the system screen) unpauses; the folder `/sdcard/OwlTransfer` exists.
- [ ] Share a photo from the emulator's gallery to Owl Transfer; it appears in the list.
- [ ] Commit any fixes: `fix(app): android storage and share sheet`.

---

## Track E: end to end

### Task E1: two desktops on Linux

Run two instances with `OWL_DATA_DIR`, `OWL_FOLDER`, `OWL_PORT`, `OWL_BEACON_PORT` overrides (add these env overrides to `settings.rs`, documented in CONTRIBUTING as the way to run two instances on one machine). Pair via nearby (the beacon on loopback), then: create, modify, delete, rename, a 50 MB file, a folder tree, conflict by pausing one (kill it, edit both, restart). Record timings. Capture `docs/img/desktop-light.webp` and `desktop-dark.webp` at 1440x860 device pixel ratio 2 (the window with a few files, one syncing).

### Task E2: Linux desktop and the Android emulator

Emulator on `tohoot_api34` or a new `owl_api34` AVD. In the app on the phone: Devices, Pair by address `10.0.2.2:52734`; accept on the desktop; then the same operations from both sides, plus the share sheet and kill/relaunch on the phone. Capture `docs/img/phone.webp` and `phone-dark.webp` (1080x2400 scaled to 780 wide).

### Task E3: release readiness

- [ ] `gh repo edit --description "A folder that is the same on your computer and your phone, synced over your own Wi-Fi in under a second. Linux, Windows and Android." --homepage https://danieltyukov.github.io/owl-transfer/`.
- [ ] Push `master`; watch CI; fix until green.
- [ ] Enable Pages via Actions (`gh api -X POST repos/danieltyukov/owl-transfer/pages -f build_type=workflow`).
- [ ] Tag `v0.1.0` after CI is green so the release workflow produces the artifacts the README and site link to. Watch it; fix and re-tag (`v0.1.1`) if a platform job fails.
