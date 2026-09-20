# The desktop end to end suite

Two copies of the real application, on one Linux machine, pairing over loopback
and syncing a folder between them. Every step of task E1 is one test: pairing,
a file each way, a rename, a tree of ten files, a delete, fifty megabytes, a
conflict after one side is killed, forgetting, reconnecting after a restart,
and the parts of the interface that only two running devices can exercise.

Nothing here is mocked. The engine is the engine, the window is the window, and
the assertions are made against the folders on disk and against the state the
engine publishes.

## Running it

```
cd app/e2e-desktop
pip install -r requirements.txt
./run.sh
```

Arguments go through to pytest: `./run.sh -k conflict` runs one step,
`./run.sh -x` stops at the first failure, and `./run.sh --keep-work` leaves the
two instances' data directories and folders behind to look at.

A run takes about two minutes, most of it the fifty megabyte file and the waits
that have to pass before a reconnection can be called late.

### What it needs

| | |
| --- | --- |
| The binary | `target/debug/owl-transfer`, from `cd app && npx tauri build --debug --no-bundle`. Point `OWL_BINARY` somewhere else to use another build. |
| `tauri-driver` | `cargo install tauri-driver --locked`. Override with `TAURI_DRIVER`. |
| `WebKitWebDriver` | On Debian and Ubuntu, `apt install webkit2gtk-driver`. |
| A desktop session | Two windows really open. A headless machine needs `xvfb-run ./run.sh`. |
| Free ports | 52734, 52735, 52744, 52745 for the two engines, and 4444, 4446, 4464, 4466 for the drivers. `OWL_E2E_PORT_BASE` moves the four engine ports, which an installed copy of the app already listening on 52734 is reason enough to do. |

`OWL_E2E_WORK` moves the two instances' directories, which default to
`/tmp/owl-e2e-desktop`. `OWL_E2E_REPORT` names a file to write the table of
steps and timings to, which is otherwise only printed.

## How it is put together

`owl.py` is one `Instance`: a data directory, a sync folder, ports, a
`tauri-driver` and a WebDriver session, plus the handful of things a test says
to a window. `conftest.py` builds the two instances once for the whole session
and prints the table at the end. `test_e1.py` is the steps.

Three things in `owl.py` are worth knowing before changing it.

Each instance needs `--native-port` of its own as well as `--port`. Without it
the second `tauri-driver` hands its sessions to the first one's
`WebKitWebDriver` and the run dies on "Maximum number of active sessions".

`element.text` is empty for every element on this WebKitWebDriver, whether or
not the window is on screen, so every comparison in the suite is against
`textContent`.

The engine's own picture is read with `get_state`, invoked in the page exactly
as the interface invokes it. That is how a step can assert something the screen
does not show, such as that a rename moved no bytes.

## The two expected failures

Two tests are marked `xfail(strict=True)`, both of them engine behaviour that
task E1 asked for and did not get:

- A device that was dialled is never told that the other side forgot it. The
  engine says so by refusing the next connection, and a device that was dialled
  stores no address for the peer, so it never dials and never hears.
- A device name changed while two devices are connected does not reach the
  other one until the next connection. The name travels in the `Hello` at the
  start of a connection and nothing re-sends it.

Strict means the run fails if either starts passing, so whoever fixes the
engine is told to delete the mark. Both are written up in the task E report.

## Leaving the machine as it was found

The fixture kills any copy of the app left over from an interrupted run,
matching on `OWL_DATA_DIR` in the process environment, since both instances run
the same binary with no arguments and `ps` cannot tell them apart. On the way
out it ends both sessions, stops both drivers and removes the working
directory, unless `--keep-work` says otherwise.
