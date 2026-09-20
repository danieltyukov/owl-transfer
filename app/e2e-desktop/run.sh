#!/usr/bin/env bash
#
# The desktop end to end suite: two real windows, paired over loopback, driven
# through WebDriver. Arguments are passed on to pytest, so
# `./run.sh -k conflict` runs one step and `./run.sh --keep-work` leaves the
# two folders behind to look at.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
binary="${OWL_BINARY:-$repo/target/debug/owl-transfer}"
driver="${TAURI_DRIVER:-$HOME/.cargo/bin/tauri-driver}"

if [ ! -x "$binary" ]; then
  echo "no binary at $binary" >&2
  echo "build one with: cd $repo/app && npx tauri build --debug --no-bundle" >&2
  exit 1
fi

if [ ! -x "$driver" ]; then
  echo "no tauri-driver at $driver" >&2
  echo "install it with: cargo install tauri-driver --locked" >&2
  exit 1
fi

if ! command -v WebKitWebDriver >/dev/null 2>&1; then
  echo "WebKitWebDriver is not on the PATH; it comes with webkit2gtk-driver" >&2
  exit 1
fi

if [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
  echo "no desktop session: this suite opens two windows" >&2
  exit 1
fi

cd "$here"
exec python3 -m pytest -v "$@"
