#!/usr/bin/env bash
#
# Task E2: a Linux desktop instance and the Android emulator, end to end.
# Arguments are passed on to the run, which takes none today.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export PATH="$ANDROID_HOME/platform-tools:$PATH"

if ! command -v adb >/dev/null 2>&1; then
  echo "adb is not on the PATH; set ANDROID_HOME" >&2
  exit 1
fi

serial="${OWL_E2E_SERIAL:-emulator-5556}"
if ! adb devices | grep -q "^$serial[[:space:]]*device$"; then
  echo "no device at $serial" >&2
  echo "start one with: emulator -avd owl_api35 -no-snapshot-load -gpu swiftshader_indirect" >&2
  exit 1
fi

if [ ! -x "${OWL_BINARY:-$repo/target/debug/owl-transfer}" ]; then
  echo "no desktop binary; build one with: cd $repo/app && npx tauri build --debug --no-bundle" >&2
  exit 1
fi

cd "$here"
exec python3 -u run_e2.py "$@"
