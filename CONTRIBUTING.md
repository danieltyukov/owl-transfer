# Contributing

Bug reports and patches are welcome. There is no CLA and no style bikeshed;
match the code that is already there.

## Layout

    crates/core/       owl-core: identity, discovery, TLS transport, the index,
                       the watcher and the sync rules
    app/src/           React 19 and Vite. The whole interface.
    app/src-tauri/     the Tauri 2 shell, with gen/android/ committed
    site/              the project site, its own Vite build
    scripts/           the font and icon pipelines, run by hand and rarely
    docs/              ARCHITECTURE, screenshots, the design and plan documents

Two seams hold the project together, and both are easy to break by accident.

`crates/core` never mentions Tauri and never touches a UI. It is a library with
one `Engine` type, a `State` snapshot stream and a set of methods, and it is
tested by starting two engines in one process on loopback. Putting platform
code in it means that test can no longer cover the thing it is testing.

`app/src` never imports a Tauri module outside `app/src/backend/tauri.ts`.
Everything else goes through the `Backend` interface in
`app/src/backend/types.ts`, which has a second implementation,
`app/src/backend/mock.ts`, holding an in-memory tree. That is what lets the
whole interface run and be tested in a plain browser with no engine behind it,
and a component that reaches for `invoke` directly breaks the test strategy
rather than just the layering.

The package manager is npm workspaces. `app` and `site` are both workspace
members, so one `npm ci` at the root covers both. Rust is a Cargo workspace at
the root holding `crates/core` and `app/src-tauri`.

## Running the tests

```
npm ci
npm run typecheck     # tsc -b across every project reference
npm test              # vitest, against the mock backend
npm run test:e2e      # playwright, against the production build
cargo test -p owl-core
cargo clippy -p owl-core --all-targets -- -D warnings
cargo fmt --all --check
```

Neither suite needs a device, a network or a build step first. The engine's
integration tests in `crates/core/tests/two_devices.rs` start two engines in
one process on `127.0.0.1` with `tcp_port: 0`, pair them, and assert on real
files in temp directories, so they are slower than the unit tests and worth
running before a push.

Playwright needs browsers once: `npx playwright install --with-deps chromium`.
The end-to-end config starts `vite preview` against a real build rather than
the dev server, because a bundling mistake that only appears in production is
exactly what a shell test should catch.

`cargo check -p owl-core --target x86_64-pc-windows-gnu` runs in CI and is
worth running locally after touching the watcher or anything that stats a file.
The Windows code paths in `notify` and `filetime` are compiled but not executed
there, which is enough to stop them rotting unseen between Windows releases.

## Running each target

**The interface on its own**, which is all you need for most changes. It starts
against the mock backend, with a folder of invented files and one paired
device, so there is nothing to pair and nothing to install:

```
npm run dev -w app
```

**Desktop.** On Ubuntu, install the toolchain first:

```
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
cd app && npx tauri dev
```

`librsvg2-dev` is only needed for the AppImage. Without it linuxdeploy's GTK
plugin exits 1 with "there is no 'libdir' variable for 'librsvg-2.0'", while
the deb still builds, which makes the failure look unrelated to the bundle
target.

**Desktop on Windows.** Needs the Rust toolchain from rustup and the Visual
Studio Build Tools with the C++ workload. Then, in a Developer PowerShell:

```
cd app; npx tauri build --bundles nsis,msi
```

`tauri.windows.conf.json` is merged over `tauri.conf.json` on Windows, which is
where the product becomes "OwlTransfer" for the installer and the Start menu.

**Android.** Needs JDK 21, the Android SDK, NDK 27 or newer, and the Rust
targets:

```
rustup target add aarch64-linux-android x86_64-linux-android
cd app && npx tauri android build --apk --target aarch64 --target x86_64
```

`ANDROID_HOME` and `NDK_HOME` have to point at the SDK and at the NDK version
you installed. `app/src-tauri/gen/android/` is committed because the manifest,
`MainActivity.kt` and the plugin class are source; everything Gradle generates
inside it is gitignored.

Three things in that project are ours rather than generated, and a regenerate
would drop them. The manifest declares `MANAGE_EXTERNAL_STORAGE` and the
`ACTION_SEND` intent filters. `MainActivity` acquires a `MulticastLock`,
without which the Wi-Fi driver filters the discovery broadcasts and Nearby
stays permanently empty, and it copies shared files into the sync folder.
`OwlPlugin` answers the two permission commands.

## Two instances on one machine

The engine itself reads no environment, because it takes a `Config` and that is
the seam described above. The Tauri shell's settings module,
`app/src-tauri/src/settings.rs`, reads four overrides and passes them into that
`Config`, so that sync can be exercised without a second device:

| Variable | Default | What it does |
| --- | --- | --- |
| `OWL_DATA_DIR` | the platform app data directory | Where `device.json`, `peers.json`, `index.json` and `settings.json` go |
| `OWL_FOLDER` | `~/OwlTransfer` | The folder to sync |
| `OWL_PORT` | `52734` | The TCP port this instance listens on |
| `OWL_BEACON_PORT` | `52735` | The UDP discovery port. `0` disables the beacon |

`OWL_DATA_DIR` is the important one: the certificate lives there, so two
instances sharing it are one device as far as the protocol is concerned, and
nothing will work. Give the second instance its own directory, its own folder
and its own TCP port:

```
OWL_DATA_DIR=/tmp/owl-b \
OWL_FOLDER="$HOME/OwlTransferB" \
OWL_PORT=52744 \
OWL_BEACON_PORT=0 \
  ./owl-transfer
```

The beacon is off on the second instance because two sockets bound to the same
UDP port on one machine share the incoming packets between them rather than
both receiving every one, so discovery on loopback is unreliable in a way that
looks like a bug in discovery. Pair by address instead: on the second instance,
Devices, Pair by address, `127.0.0.1` and `52734`, then accept on the first.
One side dialling is enough, and the connection carries the folder both ways.

**Against the Android emulator.** The emulator sits behind a NAT, so its
broadcasts never reach the host and Nearby stays empty on both sides. The host
is reachable from inside the emulator at `10.0.2.2`, so pair from the phone:
Devices, Pair by address, `10.0.2.2` and `52734`, then accept on the desktop.

## Where the app keeps its state

| Platform | Directory |
| --- | --- |
| Linux | `~/.local/share/com.owltransfer.app/` |
| Windows | `%APPDATA%\com.owltransfer.app\` |
| Android | the app's private files directory, reachable with `adb shell run-as com.owltransfer.app` |

Four files live there. `device.json` holds the certificate and its private key
and is the device's identity, so deleting it makes a new device that every peer
will refuse until it is paired again. `peers.json` is the list of fingerprints
this device trusts. `index.json` is one entry per path in the folder, rebuilt
by a rescan if it is deleted. `settings.json` holds the folder path and the
device name.

The sync folder itself is never inside that directory. Deleting the data
directory throws away the pairing and the index, and touches no files.

## Commits

Conventional commits, scoped by package: `feat(core):`, `fix(ui):`,
`feat(app):`, `docs:`, `ci:`. Describe the change and why it is right, not the
process that produced it.

No emojis, and no em or en dashes, anywhere: prose, code comments, commit
messages or documentation.

## CI

`.github/workflows/ci.yml` runs on every push to `master`, on every pull
request, and by hand from the Actions tab, in three jobs. `rust` runs
`cargo fmt --all --check`, clippy with warnings denied, the engine tests and
the Windows cross-check. `web` runs the typecheck, the unit tests, the app
build and the Playwright smoke. `site` builds the site, which is a separate job
because the site imports `app/src/tokens.css` from outside its own root: moving
or renaming that file breaks the site, and a pull request should say so rather
than the next deploy discovering it. CI needs no secrets, so it runs on forks.

Work on a branch is covered by its pull request and not by the push, which is
deliberate. With both triggers firing on every branch, a push to a branch that
had a pull request open ran the whole workflow twice, once per event, and the
two runs sat in different concurrency groups because `github.ref` differs, so
neither cancelled the other. Push a branch with nothing open against it and
nothing runs until you open one, or start a run yourself from the Actions tab.

`.github/workflows/pages.yml` builds `site/` and deploys it to GitHub Pages on
push to `master`. Pages must be set to "GitHub Actions" as its source, once, in
the repository settings; with the default "Deploy from a branch" the deploy
step fails with a 404 that does not say which setting is wrong.

`.github/workflows/release.yml` runs on a `v*` tag and attaches five artifacts
to a GitHub Release: the APK, the deb, the AppImage and the two Windows
installers. It reads four repository secrets, all of them Android signing
material:

| Secret | Value |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | The release keystore, base64 encoded: `base64 -w0 owl-transfer-release.jks` |
| `ANDROID_KEYSTORE_PASSWORD` | Store password |
| `ANDROID_KEY_ALIAS` | Key alias |
| `ANDROID_KEY_PASSWORD` | Key password |

With those unset the release still runs and produces an unsigned APK rather
than failing, so a fork can cut its own tag. The workflow writes the keystore
to a temp file and deletes it in an `always()` step; nothing about a real key
belongs in this repository, and `*.jks`, `*.keystore` and `keystore.properties`
are gitignored so one cannot arrive by accident.

Desktop release builds run on `ubuntu-22.04`, not `ubuntu-latest`. glibc pins
forward: a binary built on 24.04 fails to start on 22.04 with a missing-symbol
error, so building on the oldest base that still ships WebKitGTK 4.1 is what
makes the artifact portable. Local builds on a newer Ubuntu are fine for
development.

Keep `runs-on` to standard GitHub-hosted runners. Actions minutes are free and
unmetered on public repositories with standard runners; larger runners bill
even on a public repository.
