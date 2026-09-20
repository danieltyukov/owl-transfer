# Owl Transfer rebuild: design

Date: 2026-09-20. Status: approved for implementation (autonomous run; the
decisions below were made by the implementer and are recorded so they can be
challenged).

## What it is

A folder that is the same on your Linux or Windows desktop and your Android
phone. Drop a file in on one device and it is on the other within a second,
over the local network, with no account and no server. Both devices keep a
full copy, so either works offline and they catch up when they next see each
other.

The repository already existed as a Flutter prototype. It did not work well,
and the reasons were in the design rather than the toolkit:

- File contents were sent as base64 inside newline-delimited JSON over a raw
  TCP socket. A large file was one line; a stray newline in a path broke the
  framing; a 100 MB file was a 133 MB string in memory on both sides.
- Sync ran on a ten-minute timer and only when the user pressed a button.
  There was no file watcher, so "near instant" was structurally impossible.
- Pairing baked an IP address into a QR code. The phone's address changes
  every time it rejoins the network, so the pairing broke the next day.
- Every sync hashed every file in full (MD5, whole tree) on both sides.
- Nothing was encrypted or authenticated beyond an eight-character code sent
  in the clear, and a reconnect sent an empty code.
- There was no discovery, so the two devices could not find each other.

The rebuild keeps the goal and replaces everything else.

## Stack

**Tauri 2** for all three targets (Linux, Windows, Android), **React 19 +
Vite** for the interface, and a **Rust** crate for the sync engine that is the
same code on every platform.

Why not Flutter again: the previous attempt's problems were not Flutter's, but
two things push the choice. The reference application, `to-hoot`, is React +
Tauri, and the design system, repository layout, CI and release pipeline from
it transfer directly. And the sync engine has to be native (sockets, file
watching, hashing, TLS) on every platform; Rust with Tauri 2 mobile gives one
engine for desktop and Android, where a Capacitor shell would have needed a
second engine in Kotlin.

Why not `to-hoot`'s exact shape (Tauri desktop + Capacitor mobile): see above.
`to-hoot`'s logic is JavaScript against an HTTP API, so a WebView shell was
enough. This app's logic is a TCP server and a file watcher, which a WebView
cannot host.

## Repository layout

    crates/core/        owl-core: the engine. No Tauri, no UI. Unit and
                        integration tested on its own.
    app/                the Tauri 2 project
      src/              React 19 + Vite. The whole interface.
      src-tauri/        the Rust glue: commands, events, app lifecycle
      src-tauri/gen/android/   the generated Android project, committed
    site/               the one-page project site (GitHub Pages)
    docs/               ARCHITECTURE.md, img/, superpowers/
    scripts/            icon and font pipelines, run by hand
    .github/workflows/  ci.yml, release.yml, pages.yml

A Cargo workspace at the root holds `crates/core` and `app/src-tauri`. An npm
workspace at the root holds `app` and `site`. The Flutter tree is deleted.

## The engine (`crates/core`)

### Identity and trust

On first run the engine generates a self-signed TLS
certificate (`rcgen`, Ed25519) and a private key, stored in the app's data
directory. The certificate's SHA-256 fingerprint, hex, is the **device id**,
the thing a peer pins and the thing the interface shows (its first eight
characters). The device also has a display name
(default: the hostname on desktop, the model name on Android) and a kind
(`desktop` or `phone`).

Every connection is TLS 1.3 (`rustls` with the `ring` backend, pure Rust so
it cross-compiles to Android without a C toolchain beyond the NDK). Both sides
present their certificate. A custom verifier accepts a certificate only if its
fingerprint belongs to a paired peer, or if the connection is a pairing
attempt, in which case the handshake completes and the application layer runs
the pairing exchange below before anything else is allowed.

### Discovery

A UDP beacon on port 52735, sent every two seconds to the broadcast address of
every interface: `{"v":1,"id":..,"name":..,"kind":..,"port":52734}`. Every
device listens on the same port and keeps a table of who was heard and from
which address, expiring entries after ten seconds. No mDNS: nothing else needs
to find this service, and a broadcast is one socket with no library.

Android filters multicast and broadcast at the Wi-Fi driver unless the app
holds a `MulticastLock`; `MainActivity` acquires one.

Discovery fails on networks with client isolation and across NAT (the Android
emulator is behind one). For those there is **pair by address**: the user
types `host:port` on one device. Once connected, the connection carries sync
in both directions, so only one side ever needs to be able to dial.

### Pairing

Numeric comparison, like Bluetooth, because the person pairing holds both
devices:

1. Device B (from the nearby list, or from a typed address) connects to A over
   TLS and sends `PairRequest {id, name, kind}`.
2. Both compute a six-digit code from the two certificate fingerprints and a
   nonce A sends back: `code = HMAC-SHA256(nonce, fpA || fpB) mod 10^6`.
3. A shows "B wants to pair. Code 482 913. Accept?". B shows "Pairing with A.
   Code 482 913. Waiting". The user checks the codes match and accepts on A.
4. A sends `PairAccept`; both store the other's fingerprint, name and kind in
   `peers.json`. The connection is now a normal peer connection and sync
   starts on it.

A rejected or timed-out request (60 s) closes the connection. Forgetting a
peer removes it from `peers.json`; the next connection from it is refused at
the TLS layer.

### Connections

One TCP connection per peer, TLS on top, then length-prefixed frames
(`u32` big-endian length, one byte of frame type, payload). Control frames are
JSON. Data frames are raw bytes with a fixed header. A keepalive `Ping` every
15 s; a connection that answers nothing for 45 s is dropped.

When a beacon from a paired peer is heard and there is no connection, the
device dials it. Both may dial at once; the tie-break is that the connection
initiated by the device with the lexically smaller id survives. A paired
peer's last known address is stored and retried every five seconds while
disconnected, so pair-by-address peers reconnect without a beacon.

### The index

Each device keeps an index of the sync folder: one entry per path.

    Entry {
      path: String,            relative, forward slashes, NFC
      kind: File | Dir,
      size: u64,
      mtime_ms: i64,
      hash: Option<Blake3>,    files only
      deleted: bool,           a tombstone
      vv: VersionVector,       DeviceId -> u64
      seen_at: i64,            local wall clock, for tombstone expiry
    }

The index is persisted as a single JSON file in the app's data directory,
rewritten atomically (write to a temp file, rename) and debounced to at most
once per 500 ms. A few thousand entries is a few hundred kilobytes; this is a
shared folder between two devices, not a backup system.

### Change detection

`notify` (inotify on Linux and Android, `ReadDirectoryChangesW` on Windows)
watches the folder recursively. Events are debounced for 300 ms, then the
affected paths are rescanned: `stat`, and if size or mtime differ from the
index, the file is hashed (BLAKE3, streaming). A changed entry gets its version
vector bumped (`vv[self] += 1`) and is announced to every connected peer as an
`IndexUpdate`.

Android's shared storage is a FUSE mount and does not deliver inotify events
for changes made by other apps reliably. On Android the engine adds a
`PollWatcher` (2 s, size and mtime only) alongside inotify, and rescans on app
resume. "Near instant" on the phone therefore means within two seconds for
changes made by other apps; changes the app itself makes (share sheet, add
files) are indexed immediately.

A full rescan also runs at start, on every new connection, and every five
minutes, so a missed event can never be permanent.

Ignored: the temp files the engine writes (`.owl-tmp-*`), `.owl/`, and OS
litter (`.DS_Store`, `Thumbs.db`, `desktop.ini`, `~$*`).

### Sync

When a connection is established, both sides exchange their full `Index`.
Afterwards each sends `IndexUpdate` frames as its own entries change. On
receiving a remote entry R for a path with local entry L:

- No L: adopt R (download, create the directory, or record the tombstone).
- `R.vv` dominates `L.vv`: adopt R.
- `L.vv` dominates `R.vv`: ignore; the peer has or will get L from our index.
- Equal: nothing to do.
- Concurrent: a conflict. If either side is a tombstone, the live file wins
  (a deletion never destroys an edit). Otherwise the newer `mtime_ms` wins,
  ties broken by the larger device id, and the loser is written beside the
  winner as `name (conflict from <device> 2026-09-20 13-45).ext`, which is a
  new local change that syncs like any other. The winner's `vv` becomes the
  merge of both plus a bump.

Adopting a remote file: request its blocks (256 KiB, up to 8 in flight per
connection) with `Request {path, hash, offset, len}`; write them to
`.owl-tmp-<hash>` in the target directory; verify the whole-file BLAKE3;
set the mtime; rename into place; write the index entry with R's version
vector so the watcher's own event for the rename is recognised as already
known and does not bump it.

Before requesting from the network, look for a local file with the same
hash. A rename or move on the other side is then a local copy on this side,
which is what makes it instant.

Tombstones are kept for 30 days, then dropped.

The engine reports per-file and aggregate progress so the interface can show
"Syncing 3 files, 12 MB/s".

### Public surface

The engine is a library with one `Engine` type: `start(config)`, a
`watch_state()` stream of `State` snapshots, and methods for everything the
interface can do. `State` is the whole picture in one struct: this device,
the folder, every peer with its connection status, every nearby unpaired
device, the transfer summary, and any pending pairing request. The Tauri glue
forwards `State` to the WebView as an event and exposes the methods as
commands. Nothing in the UI holds sync state of its own.

## The Tauri app (`app/`)

### Commands

    get_state()                       -> State
    list_dir(path)                    -> [Entry with a sync badge]
    import_files(paths, into)         copies picked or dropped files in
    create_folder(path)
    delete_entry(path)
    rename_entry(path, new_name)
    open_entry(path)                  opener plugin, on desktop and Android
    reveal_folder()                   desktop: open the folder in the file manager
    set_folder(path)
    set_device_name(name)
    set_theme(theme)
    pair_with_nearby(id)
    pair_with_address(host, port)
    respond_to_pairing(id, accept)
    forget_peer(id)
    all_files_permission()            Android: granted or not
    open_all_files_settings()         Android: the system screen that grants it

### Events

    state          the full State, on every change, coalesced to 10 per second
    dir-changed    a relative directory path whose listing is stale

### Android specifics

- `MANAGE_EXTERNAL_STORAGE`, so the sync folder can be
  `/storage/emulated/0/OwlTransfer`, visible in every file manager. Until it
  is granted the app shows one card explaining why and a button to the system
  screen; sync is paused. This permission is fine for a sideloaded app and is
  the same one every file manager and Syncthing hold.
- A `MulticastLock` for discovery.
- `ACTION_SEND` and `ACTION_SEND_MULTIPLE` intent filters for `*/*`, handled
  in `MainActivity`: the shared streams are copied into the sync folder, which
  the engine then picks up. Share a photo to Owl Transfer and it is on the
  desktop.
- Files picked in the app arrive as `content://` URIs; the fs plugin opens
  them and the Rust side streams them into the folder.

### Desktop specifics

- No native decorations. The app draws its own title bar (brand, what the
  window is showing, window controls), the same shape as `to-hoot`'s.
- Drag and drop onto the window imports files.
- Single instance; a second launch focuses the first.
- Default folder `~/OwlTransfer`, created on first run.

## The interface

### Layout

Desktop: a sidebar (brand, the folder, devices with a status dot each, Pair a
device, Settings) beside the file browser, with a status strip along the
bottom ("Up to date, 2 devices" or "Syncing 3 files, 12 MB/s"). Phone: the
file browser fills the screen with a header (brand, a status pill), and
Devices and Settings are screens pushed over it. Same components in both; the
breakpoint is 900 px.

Screens:

- **Files**: breadcrumb, a list (icon, name, size, modified, sync badge:
  synced, syncing with progress, waiting, conflict), Add files, New folder,
  a drop target that lights up. Long-press or right-click for rename and
  delete. Empty state: one serif sentence and the two buttons.
- **Devices**: paired devices (name, kind, connected or last seen, Forget),
  nearby devices (name, kind, Pair), and Pair by address. A pending pairing
  request is a card at the top with the six-digit code and Accept / Decline.
- **Settings**: device name, sync folder (Change, and Open on desktop), theme
  (system, light, dark), the Android permission card, and About (version,
  the repository).
- **First run**: not a wizard. The Files screen with its empty state and a
  hint card "Pair your other device" that opens Devices.

### Design

Tokens in `app/src/tokens.css`, following `to-hoot`'s rules: the full light
palette on `:root`, dark redefined under both the media query and the explicit
attribute, no colour defined only inside a media query, two radii (6 px
controls, 10 px panels), one accent.

The palette is night and amber, which is where an owl lives: a cool, near
black ink and a warm amber accent, the pair the eyes make against the dark.
Light theme: paper `#f8f7f4`, surface white, ink `#17181a`, accent ochre
`#a35f14` (4.6:1 on white). Dark theme: ground `#111214`, surface `#191a1d`,
text `#e9e7e2`, accent amber `#e5a54a`. Green for synced, red for danger. The
accent is never used for the synced state, so the two cannot be confused.

Type: Instrument Sans for everything, JetBrains Mono for sizes, times and
codes (tabular numerals, so a progress figure does not jiggle its row), both
as OFL subsets renamed Owl Sans and Owl Mono by `scripts/fetch-fonts.sh`.

The mark: an owl's face on a 32-unit grid. Two ringed eyes (a stroke ring with
a solid pupil, which also read as two lenses, two devices), a single V brow
across the top, and a small beak. It is deliberately not `to-hoot`'s heart
brow; the two apps are siblings, not twins. The launcher and desktop icons
are rendered from the same path by `scripts/render-icons.mjs`: amber ground,
ink face. The wordmark is "Owl Transfer" in the sans.

## Testing

- `crates/core`: unit tests for the version vector, the conflict rules, the
  scanner (with a temp dir), ignore rules and frame codec; an integration test
  that starts two engines on localhost in one process, pairs them, and asserts
  a file created on one lands on the other with matching bytes and mtime, that
  a delete propagates, that a concurrent edit produces exactly one conflict
  copy, and that a 20 MB file transfers intact. The integration test measures
  create-to-visible latency and asserts it is under two seconds.
- `app/src`: Vitest with Testing Library against a mock backend (the same
  `Backend` interface the Tauri adapter implements), covering the file list,
  the pairing card, the status strip and the tokens file's invariants.
- `cargo check --target x86_64-pc-windows-gnu -p owl-core` in CI, so the
  Windows watcher path never rots unseen. The Windows installers are built by
  the release workflow on `windows-latest`; this run compiles for Windows but
  does not execute there.
- End to end, by hand in this run: two desktop instances on Linux with
  separate config directories and ports, and the Linux desktop against the
  Android emulator paired by address (`10.0.2.2`), exercising create, modify,
  delete, rename, a large file, the share sheet, and reconnection after the
  app is killed.

## Out of scope

iOS and macOS (no way to test them here; nothing in the design prevents
them). Selective sync, versioning, ignore patterns the user edits, more than
a handful of peers, relaying across the internet. Each is a later spec.
