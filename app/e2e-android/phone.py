"""One Android device or emulator, driven over adb and the debug WebView.

The app's WebView is inspectable in a debug build, so the interface can be
driven the same way the desktop suite drives its windows: find an element, read
its words, press it, and ask the engine for its state with the same `get_state`
the interface uses. `Phone.evaluate` is the whole of that seam.

Anything outside the WebView is not reachable that way. The system screen that
grants all files access belongs to Settings, not to this app, so that one is
pressed through `uiautomator`, by the words on the button.

Nothing here is a test framework. It is the handful of things the task E2 run
says to the phone, in one place, so the run reads as steps.
"""

from __future__ import annotations

import json
import re
import subprocess
import time
from pathlib import Path

import websocket

PACKAGE = "com.owltransfer.app"
ACTIVITY = f"{PACKAGE}/.MainActivity"


class Timeout(AssertionError):
    pass


def wait_until(answer, timeout: float, what: str, interval: float = 0.2):
    started = time.monotonic()
    while True:
        value = answer()
        if value:
            return value, time.monotonic() - started
        if time.monotonic() - started > timeout:
            raise Timeout(f"{what} (waited {timeout:g} s)")
        time.sleep(interval)


class Phone:
    def __init__(self, serial: str, cdp_port: int = 9333) -> None:
        self.serial = serial
        self.cdp_port = cdp_port
        self.socket: websocket.WebSocket | None = None
        self.next_id = 0

    # adb

    def adb(self, *args: str, timeout: float = 120.0) -> str:
        done = subprocess.run(
            ["adb", "-s", self.serial, *args],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        if done.returncode != 0:
            raise AssertionError(f"adb {' '.join(args)}: {done.stderr.strip()}")
        return done.stdout

    def shell(self, command: str, timeout: float = 120.0) -> str:
        """A shell command, answering with what it printed.

        `adb shell` carries the remote command's exit code back, and most of
        what this file asks is a question rather than an instruction: `pidof`
        with nothing running, `stat` on a file that has not arrived yet and
        `[ -d ... ]` all answer by failing. The caller checks the words.
        """
        done = subprocess.run(
            ["adb", "-s", self.serial, "shell", command],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return done.stdout

    def install(self, apk: Path) -> None:
        self.adb("install", "-r", str(apk), timeout=600.0)

    def uninstall(self) -> None:
        subprocess.run(
            ["adb", "-s", self.serial, "uninstall", PACKAGE], capture_output=True, text=True
        )

    def launch(self) -> None:
        self.adb("shell", "am", "start", "-n", ACTIVITY)

    def force_stop(self) -> None:
        self.adb("shell", "am", "force-stop", PACKAGE)
        self.detach()

    def home(self) -> None:
        self.adb("shell", "input", "keyevent", "KEYCODE_HOME")

    def pid(self) -> int | None:
        out = self.shell(f"pidof {PACKAGE}").strip()
        return int(out.split()[0]) if out else None

    def running(self) -> bool:
        return self.pid() is not None

    def storage_permission(self) -> str:
        """What appops says, which is not what a running process believes."""
        out = self.shell(f"appops get {PACKAGE} MANAGE_EXTERNAL_STORAGE")
        return out.strip()

    def screencap(self, into: Path) -> None:
        into.parent.mkdir(parents=True, exist_ok=True)
        done = subprocess.run(
            ["adb", "-s", self.serial, "exec-out", "screencap", "-p"], capture_output=True
        )
        into.write_bytes(done.stdout)

    # The folder on the device

    FOLDER = "/storage/emulated/0/OwlTransfer"

    def listing(self, rel: str = "") -> list[str]:
        where = f"{self.FOLDER}/{rel}".rstrip("/")
        out = self.shell(f"ls -A '{where}' 2>/dev/null")
        return sorted(line for line in out.splitlines() if line)

    def read(self, rel: str) -> bytes | None:
        done = subprocess.run(
            ["adb", "-s", self.serial, "exec-out", f"cat '{self.FOLDER}/{rel}' 2>/dev/null"],
            capture_output=True,
        )
        return done.stdout if done.returncode == 0 and done.stdout else None

    def sha(self, rel: str) -> str | None:
        out = self.shell(f"sha256sum '{self.FOLDER}/{rel}' 2>/dev/null").strip()
        return out.split()[0] if out else None

    def size(self, rel: str) -> int | None:
        out = self.shell(f"stat -c %s '{self.FOLDER}/{rel}' 2>/dev/null").strip()
        return int(out) if out.isdigit() else None

    def mtime_ms(self, rel: str) -> int | None:
        out = self.shell(f"stat -c %.3Y '{self.FOLDER}/{rel}' 2>/dev/null").strip()
        return round(float(out) * 1000) if out else None

    # The WebView

    def attach(self, timeout: float = 60.0) -> None:
        """Forwards the debug socket and opens the page's CDP connection."""
        pid, _ = wait_until(self.pid, timeout, "the app did not start")
        self.adb("forward", f"tcp:{self.cdp_port}", f"localabstract:webview_devtools_remote_{pid}")

        def page():
            try:
                import urllib.request

                with urllib.request.urlopen(
                    f"http://127.0.0.1:{self.cdp_port}/json", timeout=5
                ) as answer:
                    for item in json.load(answer):
                        if item.get("type") == "page" and item.get("webSocketDebuggerUrl"):
                            return item
            except Exception:
                return None
            return None

        found, _ = wait_until(page, timeout, "the WebView never offered a page")
        # No Origin header: the WebView's debugger refuses a connection that
        # carries one it was not started with, and this is not a browser.
        self.socket = websocket.create_connection(
            found["webSocketDebuggerUrl"], timeout=60, suppress_origin=True
        )
        self.send("Runtime.enable")
        self.wait_ready()

    def wait_ready(self, timeout: float = 60.0) -> None:
        """Waits for the interface to have drawn something.

        The page is there as soon as the activity is, and for a moment after
        that the app is an empty shell waiting for its first state: no tabs, no
        panes, nothing to press.
        """
        wait_until(
            lambda: self.evaluate("document.querySelector('.app') !== null"),
            timeout,
            "the interface never drew",
        )

    def detach(self) -> None:
        if self.socket is not None:
            try:
                self.socket.close()
            except Exception:
                pass
            self.socket = None

    def send(self, method: str, **params):
        assert self.socket is not None, "not attached to the WebView"
        self.next_id += 1
        want = self.next_id
        self.socket.send(json.dumps({"id": want, "method": method, "params": params}))
        while True:
            answer = json.loads(self.socket.recv())
            if answer.get("id") == want:
                if "error" in answer:
                    raise AssertionError(f"{method}: {answer['error']}")
                return answer["result"]

    def evaluate(self, expression: str):
        """Runs an expression in the page and answers with its value.

        Promises are awaited, so `invoke` reads as a call rather than as a
        callback.
        """
        result = self.send(
            "Runtime.evaluate",
            expression=expression,
            awaitPromise=True,
            returnByValue=True,
        )
        if result.get("exceptionDetails"):
            raise AssertionError(result["exceptionDetails"].get("text", "the page threw"))
        return result["result"].get("value")

    def invoke(self, command: str, **args):
        return self.evaluate(
            f"window.__TAURI_INTERNALS__.invoke({json.dumps(command)}, {json.dumps(args)})"
        )

    def state(self) -> dict:
        return self.invoke("get_state")

    def peers(self) -> list[dict]:
        return self.state()["peers"]

    def connected_to(self, device_id: str) -> bool:
        return any(p["id"] == device_id and p["connected"] for p in self.peers())

    def text_of(self, selector: str) -> str | None:
        return self.evaluate(
            f"(() => {{ const e = document.querySelector({json.dumps(selector)});"
            " return e === null ? null : e.textContent.trim(); })()"
        )

    def texts(self, selector: str) -> list[str]:
        return self.evaluate(
            f"[...document.querySelectorAll({json.dumps(selector)})].map(e => e.textContent.trim())"
        )

    def press(self, selector: str, text: str | None = None) -> bool:
        """Presses the first element matching, optionally by its words."""
        return bool(
            self.evaluate(
                f"(() => {{ const want = {json.dumps(text)};"
                f" const all = [...document.querySelectorAll({json.dumps(selector)})];"
                " const hit = want === null ? all[0] : all.find(e => e.textContent.trim() === want);"
                " if (hit === undefined || hit === null) return false;"
                " hit.click(); return true; })()"
            )
        )

    def fill(self, selector: str, value: str) -> bool:
        """Types into a controlled input the way React hears it.

        Setting `value` alone is invisible to React, which keeps its own
        record of what the input last had; the setter on the prototype plus an
        input event is the documented way round it.
        """
        return bool(
            self.evaluate(
                f"(() => {{ const el = document.querySelector({json.dumps(selector)});"
                " if (el === null) return false;"
                " const set = Object.getOwnPropertyDescriptor("
                "   window.HTMLInputElement.prototype, 'value').set;"
                f" set.call(el, {json.dumps(value)});"
                " el.dispatchEvent(new Event('input', {bubbles: true}));"
                " return true; })()"
            )
        )

    def tab(self, label: str) -> None:
        """Files, Devices or Settings, from the tabs along the bottom."""
        wait_until(lambda: self.press(".tab", label), 20.0, f"no tab called {label}")
        wait_until(
            lambda: self.evaluate("document.querySelector('.app').dataset.pane") == label.lower(),
            10.0,
            f"{label} did not open",
        )

    # Outside the WebView

    def ui_dump(self) -> str:
        return self.adb("exec-out", "uiautomator", "dump", "/dev/tty", timeout=60.0)

    def tap(self, x: int, y: int) -> None:
        self.adb("shell", "input", "tap", str(x), str(y))

    def tap_by_text(self, text: str, timeout: float = 20.0) -> tuple[int, int]:
        """Presses something on a system screen, by the words on it.

        The WebView cannot see the screen that grants all files access, and
        that screen is the one the permission card sends a person to.
        """

        def spot():
            dump = self.ui_dump()
            for match in re.finditer(r'<node[^>]*?>', dump):
                node = match.group(0)
                label = re.search(r'text="([^"]*)"', node)
                if label is None or label.group(1).strip().lower() != text.lower():
                    continue
                box = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', node)
                if box is None:
                    continue
                left, top, right, bottom = (int(n) for n in box.groups())
                return ((left + right) // 2, (top + bottom) // 2)
            return None

        at, _ = wait_until(spot, timeout, f"nothing on screen said {text!r}")
        self.tap(*at)
        return at
