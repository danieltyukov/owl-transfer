# Security

## Reporting

Use GitHub's private vulnerability reporting: the Security tab of
<https://github.com/danieltyukov/owl-transfer>, then "Report a vulnerability".
That opens a private thread visible only to the maintainers, which is the right
place for anything you would not want in a public issue.

This is one person's side project with no service behind it and no on-call
rotation. Expect a reply in days, not hours. There is no bounty.

If the report is about your own installation rather than about this code, act
first and report second. Forgetting a peer on the Devices screen stops that
device connecting from the next attempt onward, and deleting the data directory
destroys the certificate the device was identified by. Neither needs anybody
else's cooperation, because there is no account and no server holding anything
on your behalf.

## Where the keys live

There is one secret in this app, and it never leaves the device that made it.

| Secret | Stored in | What it is for |
| --- | --- | --- |
| The device's TLS private key | `device.json` in the data directory | Authenticating this device to the peers that paired with it |
| The certificate it belongs to | The same file | Its SHA-256 is the device id, which is what peers pin |
| The list of trusted fingerprints | `peers.json` in the data directory | Deciding which certificates are allowed to connect |

The data directory is `~/.local/share/com.owltransfer.app/` on Linux,
`%APPDATA%\com.owltransfer.app\` on Windows, and the app's private files
directory on Android.

There is no account, no token, no password and no telemetry. Nothing is
uploaded anywhere, because there is nowhere for it to go: the only network
traffic this app makes is a UDP broadcast on your own network advertising that
it exists, and TLS connections to devices you have paired with and to a device
you are in the middle of pairing with.

Anyone who can read the data directory holds the private key and can act as
that device until the peer forgets it. That is the same exposure as an SSH key
on the same machine, and it is protected the same way: file permissions on the
desktop, and app-private storage on Android. The same directory holds
`index.json`, which lists every path, size and hash in the sync folder, so it
discloses what the folder contains even to someone who cannot read the files.

Nothing in the sync folder is encrypted. It is ordinary files in an ordinary
directory, which is the point of the app, and full-disk encryption is what
covers them at rest.

## The pairing code

The six digits are not a password and there is nothing to type. Both devices
compute them from the two certificate fingerprints and a fresh nonce the
accepting side generates:

```
mac  = HMAC-SHA256(key = nonce, msg = fp_acceptor_hex || fp_requester_hex)
code = u32::from_be_bytes(mac[0..4]) % 1_000_000
```

They are a comparison, and what they defend is the one moment when neither
device knows the other yet. Anything sitting between the two has to hold a
separate TLS session with each of them, presenting its own certificate on both,
so the fingerprints going into the two calculations differ and the two screens
show different numbers. The person holding both devices is the check.

Because it is derived rather than chosen, there is nothing to guess: an
attacker cannot try codes, only make one attempt whose number will not match.
A declined request, or sixty seconds without an answer, closes the connection.

After pairing, trust is the stored fingerprint and nothing else. Every
connection is checked against `peers.json` during the TLS handshake, so
forgetting a peer takes effect on its next attempt rather than at some later
point.

A device you paired with can read and write everything in the sync folder.
There is no read-only peer and no partial access. That is what pairing means,
and it is worth being deliberate about which devices you pair.

## Android permissions

The manifest declares six, and each one is doing something specific:

| Permission | Why |
| --- | --- |
| `INTERNET` | Opening and accepting TCP connections on the local network. Android requires it for any socket, local or not |
| `ACCESS_NETWORK_STATE`, `ACCESS_WIFI_STATE` | Knowing which interfaces exist, to send the discovery beacon to the right broadcast addresses |
| `CHANGE_WIFI_MULTICAST_STATE` | Taking a `MulticastLock`. Without it the Wi-Fi driver drops incoming broadcasts and the phone never sees another device |
| `MANAGE_EXTERNAL_STORAGE` | Reading and writing the sync folder at `/storage/emulated/0/OwlTransfer`, where every file manager can see it |
| `READ_EXTERNAL_STORAGE`, capped at API 32 | The same folder on Android 12 and older, which predate the permission above |
| `WRITE_EXTERNAL_STORAGE`, capped at API 29 | The same, on Android 10 and older |

The last two carry `android:maxSdkVersion`, so the system stops granting them
above those levels. On a phone running anything current they are inert: they do
not appear in the app's permission list and they are never requested. They are
there because the app supports back to API 26, where the storage model was the
older one.

`MANAGE_EXTERNAL_STORAGE` is the broad one and it deserves the explanation.
Android grants it through a system settings screen rather than a dialog, and it
gives the app access to shared storage as a whole rather than to one folder,
because Android has no permission that means "this directory". The alternatives
were worse: the app's private directory is invisible to every other app on the
phone, and the document picker cannot give a background service a folder it can
watch. Sync stays paused until the permission is granted, and the app says why
rather than asking silently.

The consequence runs the other way too. The sync folder is in shared storage,
so every other app on the phone can read it. Shared storage is shared, and that
is the trade for the folder being usable from the gallery and from a file
manager.

The app also declares `ACTION_SEND` and `ACTION_SEND_MULTIPLE` intent filters,
which is what puts it in the share sheet. Sharing to it copies the shared
streams into the sync folder and nothing else.

## Releases

Release binaries are built by `.github/workflows/release.yml` on a tag, on
GitHub-hosted runners, from the commit the tag points at. The workflow is in
the repository and the build log is public, so you can check what went into an
artifact before you install it.

The Android keystore reaches CI as a base64 repository secret, is written to a
temp file, and is removed in an `always()` step. It is not in the repository
and never has been, and `*.jks`, `*.keystore` and `keystore.properties` are
gitignored so one cannot arrive by accident. If that key is ever lost, updates
can no longer install over an existing installation, and recovering means
uninstalling, which destroys the app's data directory and therefore its
identity and pairings. The sync folder survives, because it is not the app's
private storage.

The desktop installers are not code-signed. Windows SmartScreen will warn once
on the NSIS installer, and there is nothing this project can do about that
short of buying a certificate.

Releases cut from this repository are signed with the maintainer's key, which
is not Google's, so an update installs over an existing one only if it came
from the same place.

A build made without the signing secrets, which is what a fork gets, produces
an unsigned APK under the same asset name rather than failing the job, and the
run carries a warning saying so. Sign it yourself before handing it to a phone,
because Android refuses an APK with no signature at all. Once it is signed with
a different key it is still not interchangeable with a release from here: an
APK signed by one key cannot install over one signed by another, in either
direction. Crossing between them means uninstalling, which destroys the data
directory and with it the device's identity and its pairings.

## Scope

In scope: anything that lets a device you have not paired with read your files,
write into your folder or be accepted as a peer. Anything that lets a paired
peer escape the sync folder, such as a path that is written outside it.
Anything that makes the pairing code fail to detect an intercepted handshake.

Out of scope: an attacker who already has your device unlocked, or who can
already read your data directory. Both are game over by construction, and this
project does not pretend otherwise.

Also out of scope: denial of service on the local network. Anything on your
Wi-Fi can open connections that die at the handshake, and anything can flood
the discovery port. It costs CPU and it can stop two of your devices finding
each other. A design that prevented it would need an authenticated
network, which is not something an app can supply.
