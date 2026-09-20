# Architecture

This describes what Owl Transfer sends over your network, what it writes to
disk, and how it decides which copy of a file is the right one. It exists
because the app moves your files between your devices with no service in the
middle: you should be able to check what it does before you trust a folder to
it.

The implementation is `crates/core/src/`. Where this document and that code
disagree, the code is right and the disagreement is a bug worth reporting.

## The shape of it

There is no server, and there is no coordinator. Two devices that have been
paired open one connection to each other and exchange two things: what each of
them has, and the bytes the other is missing. Everything else is local.

The engine is a Rust library, `crates/core`, with one `Engine` type. It owns
the folder, the index, the connections and the peer list, and it publishes a
single `State` snapshot that describes the whole picture: this device, the
folder, every peer and whether it is connected, every unpaired device heard
nearby, any transfer in flight, and any pairing request waiting for an answer.
The interface subscribes to that stream and holds no sync state of its own,
which is why the desktop and the phone cannot disagree about what is happening.

The same library runs on Linux, Windows and Android. `app/src-tauri` is a thin
shell that starts it, forwards `State` to the WebView as an event, and exposes
its methods as commands.

## Identity

On first run a device generates a self-signed TLS certificate with an Ed25519
key and writes both to `device.json` in its data directory. Nothing else issues
it, and there is no certificate authority anywhere in this design.

The **device id** is the SHA-256 of that certificate's DER encoding, as 64
lowercase hex characters. It is the only name that matters: it is what a peer
pins, what the beacon advertises, what the version vectors are keyed by, and
what the Devices screen shows the first eight characters of. A device also has
a display name, which defaults to the hostname on the desktop and to "Android
phone" on a phone, where there is no hostname worth showing anyone, and a kind,
`desktop` or `phone`. Both are cosmetic and can change without breaking
anything.

Deleting `device.json` makes a new device. The old id is gone, every peer still
holds the old fingerprint, and the new one has to be paired again.

## Discovery

Every two seconds each device sends a UDP datagram to the broadcast address of
every interface it has, on port 52735:

```json
{ "v": 1, "id": "3f2a...", "name": "thinkpad", "kind": "desktop", "port": 52734 }
```

Every device listens on the same port, drops packets carrying its own id, and
keeps a table of who was heard and from which source address. An entry that has
not been refreshed for ten seconds is dropped, so a device that leaves the
network disappears from Nearby rather than lingering as something you can try
to pair with.

The beacon is a hint and nothing more. Anything on the network can send one
claiming any id, name and port. Acting on a beacon means dialling that address
and attempting a TLS handshake, which fails unless the certificate on the other
end hashes to a fingerprint this device already trusts. A forged beacon
therefore costs a failed connection attempt and nothing else.

There is no mDNS. Nothing outside this app needs to find this service, and a
broadcast is one socket with no dependency.

Android filters broadcast and multicast traffic in the Wi-Fi driver unless the
app holds a `MulticastLock`, so `MainActivity` takes one. Without it the phone
sends beacons that the desktop receives and receives none itself, which looks
like a one-directional network fault.

Discovery does not work on networks with client isolation, which guest and
hotel Wi-Fi usually enable, and it does not cross a NAT, which is what the
Android emulator sits behind. For both cases there is pair by address: type
`host:port` on one device. Only one side ever has to be able to dial, because a
connection carries the folder in both directions once it is up.

## Pairing

The two devices are in front of the same person, so the check is a number
comparison, the same shape Bluetooth uses.

Both devices contribute a random value to the code, and each one is committed
to before the other is known. That ordering is the whole design, and the reason
for it is in the threat model below.

1. B picks a random sixteen-byte nonce and connects to A over TLS. Each side
   checks the other against the certificate presented on that connection. B
   checks that A's certificate hashes to the id the person picked from the
   nearby list, and drops the connection if it does not. Pair by address has no
   id to check against, which is why the code is the whole check there. B then
   sends `PairRequest { id, name, kind, commit }`, where `commit` is the
   SHA-256 of that nonce and nothing else, and A refuses the request unless the
   `id` in it is the one A's view of B's certificate hashes to. Neither side
   can claim an id it holds no private key for.
2. A picks its own sixteen-byte nonce and answers `PairChallenge { nonce }`.
3. B answers `PairReveal { nonce }` with the nonce it committed to in step 1.
   A hashes it and compares against the commitment. A mismatch drops the
   connection with no code shown, because the only reason to reveal a different
   nonce is to steer the code.
4. Both sides now hold both nonces and both fingerprints, and compute the same
   six digits:

   ```
   mac  = HMAC-SHA256(key = nonce_A || nonce_B, msg = fp_A || fp_B)
   code = u32::from_be_bytes(mac[0..4]) % 1_000_000
   ```

   shown as two groups of three, `482 913`. A is the side that received the
   request and B the side that sent it, in the nonces and the fingerprints
   alike, so both sides order the inputs the same way.
5. A shows "B wants to pair. Code 482 913." and B shows the same code with
   "Waiting". The person checks that they match and accepts on A.
6. A sends `PairAccept`. Both write the other's fingerprint, name and kind into
   `peers.json`, and the connection becomes an ordinary peer connection. Sync
   starts on it immediately, with no second handshake.

A rejection, or sixty seconds of silence, closes the connection.

Forgetting a peer removes it from `peers.json`, and any frame still arriving
from it is dropped from that moment. Its next connection completes the TLS
handshake, because an unknown certificate has to be allowed that far for
pairing to be possible at all, and is then answered with
`PairReject { reason: "not paired" }` and closed.

A device that receives that reject from a peer it still has in its own list
forgets that peer too, and says why. Forgetting is a local action on one
device, so without this the other side would keep a dead entry and redial it
every five seconds forever.

## Connections

One TCP connection per peer, on port 52734, with TLS 1.3 over it. Both sides
present a certificate and both sides verify: the custom verifier computes the
SHA-256 of the presented end-entity certificate and checks it against
`peers.json`.

A certificate that is not on that list still completes the handshake, because a
device that has never paired has no other way to reach the pairing exchange.
What it does not get is a session: such a connection is allowed exactly one
thing, the exchange above, and every other frame on it is dropped until pairing
succeeds. The same applies the instant a peer is forgotten, so a connection
that was live when you pressed Forget stops being one.

`rustls` with the `ring` backend, with no system trust store and no CA
validation, because neither would mean anything here. The pinned fingerprint is
the entire trust decision. TLS 1.2 is compiled out.

Inside the connection, every message is a length-prefixed frame: a `u32` length
in network order, then a payload whose first byte says what it is. `0` is a
control frame and the rest is JSON. `1` is a block of file data, followed by
the request id, a status byte and the bytes themselves. A frame longer than
4 MiB is a protocol error and drops the connection, which bounds what a peer
can make this device allocate.

The control frames are:

| Frame | Meaning |
| --- | --- |
| `Hello` | id, name, kind and protocol version, sent by both sides once |
| `PairRequest`, `PairChallenge`, `PairReveal`, `PairAccept`, `PairReject` | the exchange above |
| `Index` | the sender's entire index, sent once per connection |
| `IndexUpdate` | entries that have changed since |
| `Request` | a byte range of one file, by path and hash |
| `Ping`, `Pong` | keepalive, every fifteen seconds; silence for forty-five drops the connection |
| `Error` | a message the other side could not act on |

When a beacon from a paired peer arrives and there is no connection to it, the
device dials. Both sides may dial at the same moment and end up with two
connections; the tie-break is that the one initiated by the device with the
lexically smaller id survives. A paired peer's last known address is stored and
retried every five seconds while it is disconnected, which is what lets a
pair-by-address peer reconnect with no beacon at all.

## The index

Each device keeps one entry per path in the folder, including directories and
including paths that have been deleted.

```rust
struct Entry {
    path: String,               // relative, '/' separated, NFC
    kind: EntryKind,            // File or Dir
    size: u64,
    mtime_ms: i64,
    hash: Option<String>,       // BLAKE3, hex, files only
    deleted: bool,              // a tombstone
    vv: BTreeMap<String, u64>,  // device id -> counter
    seen_at_ms: i64,
}
```

Paths are relative to the folder, use forward slashes on every platform, and
are normalised to Unicode NFC. Without that last step a file named with a
combining accent on one platform and a precomposed one on another is two
different paths, and the folders never converge. Anything with an empty
component, a `..`, a leading slash, a backslash or a NUL byte is rejected on
arrival, which is what stops a peer writing outside the folder.

The whole index is one JSON file, `index.json`, rewritten atomically to a temp
file and renamed, at most once every 500 ms. A few thousand entries is a few
hundred kilobytes. This is a shared folder between two devices, not a backup
system, and a database would be machinery for a problem that does not exist
here. Deleting `index.json` costs a full rescan and nothing else.

These names are never synced, at any depth: `.owl`, anything beginning with
`.owl-tmp-`, `.DS_Store`, `Thumbs.db`, `desktop.ini`, anything beginning with
`~$`, and files ending `.crdownload` or `.part`. A `.owl-tmp-` name is a
download in flight, `.owl` is reserved for the engine rather than written by it
today, and the rest are litter and half-written downloads that only ever cause
pointless transfers.

### Version vectors

`vv` maps device id to a counter, and it is how the two devices work out
whether one copy of a file descends from the other or whether they diverged.

Comparing two vectors gives one of four answers. They are equal. One dominates,
meaning every counter in it is at least the other's and one is greater. It is
dominated, the mirror of that. Or they are concurrent, meaning each has a
counter the other does not, so neither version was made with knowledge of the
other.

Merging is the pointwise maximum. Bumping sets this device's counter to one
more than the largest counter anywhere in the vector, rather than one more than
its own. A device that has been offline for a week therefore jumps straight
ahead of the other's numbering instead of producing a value that looks stale.

A version vector is not a clock. It records what a version knew, which is why
it can say "these two diverged" where a timestamp can only say which one was
written later.

## Change detection

`notify` watches the folder recursively: inotify on Linux and Android,
`ReadDirectoryChangesW` on Windows. Events are collected for 300 ms and then
the affected paths are rescanned.

A rescan stats each path. If the size and the mtime both match the index, the
file is not read at all, which is what keeps a folder of large files cheap. If
either differs, the file is hashed with BLAKE3 in a streaming read. Only if the
hash is different from the index does the entry get a bumped version vector and
go out as an `IndexUpdate`. A file whose contents are unchanged but whose mtime
moved, which is what `touch` and many editors' save paths produce, has its
mtime recorded and nothing else: no bump, no announcement, no transfer.

A path that has disappeared becomes a tombstone, along with everything under it
if it was a directory. Tombstones are kept for thirty days and then dropped,
which is long enough for a device that was switched off to come back and learn
about the deletion, and short enough that the index does not grow without
limit.

A full rescan runs at startup, on a new connection but at most once every half
minute across all peers, and every five minutes, so a missed event can be late
but cannot be permanent. The half minute is what keeps a flapping link from
holding the scanner open.

Android needs a second mechanism. Shared storage is a FUSE mount, and it does
not reliably deliver inotify events for changes other apps make. The engine
therefore also runs a polling watcher on a two-second interval, comparing size
and mtime only. It runs with the process, so a phone that froze the app catches
up on the first poll after Android thaws it. So "within a second" on the phone
means within two seconds for a file another app wrote, and immediately for
anything the app did itself, such as a file arriving through the share sheet.

## Sync

When a connection comes up, both sides send their entire `Index`. After that
each sends `IndexUpdate` frames as its own entries change. Nothing is ever
requested wholesale: a device acts on entries.

For a remote entry R arriving for a path where the local index holds L:

| Situation | What happens |
| --- | --- |
| No L | Adopt R: download the file, create the directory, or record the tombstone |
| `R.vv` dominates `L.vv` | Adopt R |
| `L.vv` dominates `R.vv` | Ignore. The peer has our version already or is about to |
| Equal | Nothing to do |
| Concurrent | A conflict, resolved below |

A conflict is resolved in this order. If exactly one side is a tombstone, the
live file wins, whatever the vectors say, and the deletion is discarded: a
delete that happened without knowledge of an edit must never destroy that edit,
and recreating a file you meant to delete is a smaller harm than losing one you
meant to keep. If both sides are live files with the same hash, there is
nothing to resolve and the merged vector is stored with no transfer. Otherwise
the newer `mtime_ms` wins, with the larger device id breaking an exact tie, and
the winner's vector becomes the merge of both plus a bump.

The losing copy is not thrown away. The device holding it renames it, keeping
the extension, to a name carrying the device the copy came from and a
timestamp:

    notes (conflict from thinkpad 2026-09-20 14-11).md

That rename is an ordinary local change and syncs like any other, so both
devices end up holding both files under the same two names. Deciding between
them is yours.

### A worked conflict

A laptop, id beginning `3f2a`, and a phone, id beginning `91c4`, both hold
`notes.md` with `vv = { 3f2a: 4, 91c4: 2 }`. The phone goes out of range.

The laptop edits the file at 13:45. Its counter becomes one more than the
largest in the vector, so `vv = { 3f2a: 5, 91c4: 2 }`.

The phone edits its copy at 14:02. By the same rule, `vv = { 3f2a: 4, 91c4: 5 }`.

They reconnect at 14:11 and exchange indexes. Comparing the two vectors: the
laptop's is higher for `3f2a` and lower for `91c4`, so neither dominates. They
are concurrent, and the hashes differ, so it is a real conflict.

Neither side is deleted, so mtime decides: 14:02 beats 13:45 and the phone's
copy wins. The laptop adopts the phone's bytes as `notes.md` and renames what
it had to `notes (conflict from thinkpad 2026-09-20 14-11).md`. That new file
is a new local change, so it syncs to the phone in the ordinary way, and both
devices end up listing two files: `notes.md` holding the phone's version, and
the conflict copy holding the laptop's.

The vectors settle a beat later. Each side stores the winner as the merge of
both vectors plus a bump of its own counter, so the laptop holds
`{ 3f2a: 6, 91c4: 5 }` and the phone holds `{ 3f2a: 5, 91c4: 6 }`, which are
concurrent again. They are also identical bytes, which is the case the same-hash
rule above covers: the next exchange merges them to `{ 3f2a: 6, 91c4: 6 }` and
transfers nothing. From there the file looks settled to a later reconnection and
to any third device.

Note what did not happen: no prompt, no blocked sync, and nothing waiting for
an answer. Both versions are on both devices, named so you can tell which is
which, and the sync carried on.

## Transfers

Adopting a remote file starts with a local lookup. If any live file in the
index already has that hash, the bytes are copied from it rather than
requested. A rename or a move on the other device is therefore a local copy on
this one, which is why moving a folder of photos around costs nothing.

Otherwise the file is pulled over the connection. The device issues
`Request { req_id, path, hash, offset, len }` frames for 256 KiB blocks, up to
eight outstanding at a time, and writes each block at its offset into
`.owl-tmp-<hash>` in the destination directory. The temp file is in the
destination directory rather than somewhere central so that the final move is a
rename within one filesystem, which is atomic, rather than a copy across a
mount boundary.

When every block has arrived, the whole file is hashed and compared with the
hash that was requested. Only then is the mtime set and the file renamed into
place. A mismatch throws the temp file away; a partial file is never visible
under the real name, and the watcher never sees a half-written one because the
`.owl-tmp-` prefix is ignored.

The index entry is written with the remote's version vector, unchanged, before
the rename is processed. That is what stops the watcher's own event for the
rename being read as a new local edit and bounced back at the peer as another
change.

The serving side answers a `Request` by reading the file at the offset. If the
file's current hash no longer matches the one asked for, because it changed
again while the transfer was running, the block comes back with a status byte
saying unavailable and the download is abandoned. The newer version is already
on its way as an `IndexUpdate`.

Progress is reported per file and in aggregate, which is what the status strip
reads to say "Syncing 3 files, 12 MB/s".

### What the badges mean

The status beside a file in the list is a local judgement, not an
acknowledgement from the peer, and it is worth knowing which is which. No peer
connected shows "only here", because nothing can be claimed about a device that
is not there. A transfer in flight shows a progress ring. An entry that changed
in the last two seconds while a peer is connected shows as waiting. A name
matching the conflict pattern shows as a conflict. Everything else shows as
synced.

The engine does not track per-file acknowledgements from the other side, so
"synced" means this device has announced the entry and has no reason to believe
otherwise. Adding acknowledgement tracking would make the badge stronger and
the protocol chattier, and for two devices on one network the difference is
about two seconds.

## What is stored where

The app keeps its own state in one directory, which differs per platform:

| Platform | Directory |
| --- | --- |
| Linux | `~/.local/share/com.owltransfer.app/` |
| Windows | `%APPDATA%\com.owltransfer.app\` |
| Android | the app's private files directory |

Four files live in it. `device.json` is the certificate and its private key. It is the device's
identity and the only thing in here that is a secret. `peers.json` is the list
of fingerprints this device trusts, with each peer's name, kind and last known
address. `index.json` is the index described above, which means it is a
complete listing of the folder: every path, size and hash, including
tombstones. `settings.json` holds the folder path and the device name.

The sync folder itself is separate and is never inside the data directory. On
the desktop it defaults to `~/OwlTransfer`. On Android it defaults to
`/storage/emulated/0/OwlTransfer`, in shared storage rather than in the app's
private area, which is the point: the folder is meant to be visible to every
file manager and gallery on the phone. That is what `MANAGE_EXTERNAL_STORAGE`
buys, and it is the same permission a file manager or Syncthing holds.

Nothing is encrypted at rest. The folder is ordinary files, which is the whole
idea, and the index is plain JSON beside them.

## Threat model

**An attacker on your network who can watch and inject traffic** cannot read
your files or write into your folder. Every connection is TLS 1.3 with both
sides authenticated, and the certificates are pinned by fingerprint, so there
is no certificate authority to mislead and no name to spoof. A certificate that
is not in `peers.json` gets a connection that can do nothing but attempt to
pair.

**An attacker who is present when you pair** is the case the six-digit code
exists for. Sitting between the two devices means relaying: holding two
separate TLS sessions, one to each, and presenting its own certificate on both.
The fingerprints going into the two codes are then different, so the two
screens disagree and the person comparing them sees it.

That check only holds because neither side can choose its contribution after
seeing the other's. A relay that could pick its nonces last would not need to
break anything: it would wait until it held both real nonces, then search its
own two for a pair that makes the two codes collide. Six digits is a target of
one in a million, which is a fraction of a second of hashing, and both screens
would show the same number while the attacker sat in the middle.

Committing first is what removes that. The requester hashes its nonce and sends
the hash before the acceptor's nonce exists, and the acceptor sends its nonce
before the requester's is revealed. Each side is bound to a value chosen while
the other was still unknown, so a relay cannot steer either code and is left
with the same one in a million, once, with a person looking at both screens.
Revealing a nonce that does not match the commitment drops the connection
before any code is shown, so there is no second attempt to grind either.

**A device you paired** can read and write everything in the folder. There is
no partial trust and no read-only peer; pairing means exactly this. If a device
is lost or is no longer yours, forget it. Frames from it stop being accepted
immediately, and its next connection is told it is not paired.

**Anyone who can read the data directory** has your private key and can
impersonate the device until the peer forgets it. They can also read
`index.json`, which lists every path and hash in the folder even for files they
cannot open. On Android that directory is private to the app; on the desktop it
is protected by nothing more than file permissions, like an SSH key.

**Other apps on the phone** can read and write the sync folder, because shared
storage is shared. That is the trade for the folder being usable from the
gallery and from every file manager. Anything you would not put in your
Downloads folder does not belong here either.

**A peer sending malformed data** is bounded. Frames are capped at 4 MiB, paths
are validated before they are used to open anything, and file contents are
verified against a BLAKE3 hash before they are given the real name. An entry
naming a path outside the folder is rejected rather than clamped.

**Denial of service is not defended.** Anything on the network can open TCP
connections that die at the handshake, and anything can flood the beacon port.
Both cost CPU. A LAN attacker who wants to stop two of your devices talking can
do so, and no design here would prevent that.

Out of scope, and treated as already lost: an attacker with your device
unlocked, and an attacker who has your data directory. Both are game over by
construction.
