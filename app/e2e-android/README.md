# The Android end to end run

A Linux desktop instance and the Android emulator, paired by address and put
through everything task E2 asks for: a fresh install and the storage permission
card, pairing, a file each way, a photo shared from the gallery, an edit, a
rename and a delete through the phone's own menus, a folder, twenty megabytes
each way with the rate, killing the app and catching up, and what happens while
the phone is on the home screen.

```
cd app/e2e-android
./run.sh
```

It prints a line per step as it goes and a table at the end, and writes the
same table to `OWL_E2E_REPORT` or to the working directory.

This is a run rather than a suite. Each step is timed and asserted, but they
are one story: a failure stops the run, because everything after it stands on
what it was supposed to leave behind. Where the desktop suite is meant to be
run again and again, this one is meant to be run when the Android side changes.

## What it needs

| | |
| --- | --- |
| A device | `emulator -avd owl_api35 -no-snapshot-load -gpu swiftshader_indirect`. Another serial goes in `OWL_E2E_SERIAL`. |
| The APK | `cd app && npx tauri android build --debug --target x86_64`. A debug build, because the WebView is only inspectable in one. |
| The desktop binary | `cd app && npx tauri build --debug --no-bundle`, or `OWL_BINARY`. |
| `tauri-driver` and `WebKitWebDriver` | The desktop side is the desktop suite's `Instance`; see `app/e2e-desktop/README.md`. |
| Python | `pip install websocket-client`, plus the desktop suite's `requirements.txt`. |

## How the phone is driven

`phone.py` is one device. Three ways in, and each is there because the other
two cannot reach:

- **adb** for the device and for the sync folder. The folder is in shared
  storage, so the run can write into it and read out of it exactly as another
  application would, which is also what exercises the poll the engine keeps
  beside inotify there.
- **The debug WebView over CDP** for the interface. `adb forward` onto the
  WebView's debug socket, then `Runtime.evaluate` in the page: the same
  `get_state` the interface uses, and `.click()` on the same buttons a finger
  would. The debugger refuses a WebSocket that carries an `Origin` header, so
  the connection is opened without one.
- **`uiautomator`** for the one screen that is not this app's. All files access
  is granted on a system settings screen, which the WebView cannot see, so that
  button is found by its words in a UI dump and pressed by coordinates.

Two things are worth knowing before changing it. `adb shell` carries the
remote command's exit code back, and most of what this file asks is a question
rather than an instruction, so `Phone.shell` reads the output and leaves the
code alone. And the WebView holds no focus until something has been tapped, so
`element.focus()` and `element.blur()` do nothing: a field that commits on blur
cannot be driven this way, and the run uses the command behind it instead.

## Leaving the emulator as it was found

The run finishes by removing the sync folder and the pushed picture, deleting
the MediaStore row for it, uninstalling the app and putting the appops entry
back to `default`. Everything it did is in `/sdcard/OwlTransfer`, in
`/sdcard/Pictures/kestrel.png` and in the app's own data.
