"""Two Owl Transfer desktop instances, driven through WebDriver.

One `Instance` is one running copy of the app: its own data directory, its own
sync folder, its own ports, its own `tauri-driver` and its own WebDriver
session. Everything the tests need to say to the app goes through here, so the
tests read as steps rather than as plumbing.

Three things are worth knowing before changing this file.

`tauri-driver` fronts `WebKitWebDriver`, and each one needs a port of its own
for both halves: without `--native-port` the second `tauri-driver` hands its
sessions to the first one's `WebKitWebDriver`, which answers "Maximum number of
active sessions".

`element.text` is empty on this WebKitWebDriver, for every element, whether or
not the window is on screen. `textContent` is what carries the words, so
`text_of` is what the tests compare against and nothing calls `.text`.

The engine's own picture of the world is read with `get_state`, invoked in the
page the way the interface invokes it. That is how a test can assert on
something the screen does not show, such as whether a rename moved any bytes.
"""

from __future__ import annotations

import json
import os
import signal
import socket
import subprocess
import time
from pathlib import Path

from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.remote.webelement import WebElement

REPO = Path(__file__).resolve().parents[2]
BINARY = Path(os.environ.get("OWL_BINARY", REPO / "target" / "debug" / "owl-transfer"))
TAURI_DRIVER = Path(os.environ.get("TAURI_DRIVER", Path.home() / ".cargo" / "bin" / "tauri-driver"))

# How long each kind of wait is given. Generous on purpose: the point of a
# timeout here is to fail rather than hang, not to measure anything. What the
# step took is recorded separately by whatever waited.
STARTUP = 45.0
UI = 15.0
SYNC = 20.0
BIG_SYNC = 240.0


class Timeout(AssertionError):
    """A wait that ran out. An assertion error, so pytest reports it as one."""


def wait_until(answer, timeout: float, what: str, interval: float = 0.05):
    """Polls `answer` until it returns something truthy.

    Returns the pair (what it returned, how long it took in seconds), so a
    caller can both use the value and record the time.
    """
    started = time.monotonic()
    while True:
        value = answer()
        if value:
            return value, time.monotonic() - started
        if time.monotonic() - started > timeout:
            raise Timeout(f"{what} (waited {timeout:g} s)")
        time.sleep(interval)


def mtime_ms(path: Path) -> int:
    """A file's modified time in whole milliseconds, the way the engine keeps it."""
    return path.stat().st_mtime_ns // 1_000_000


def set_mtime_ms(path: Path, when_ms: int) -> None:
    os.utime(path, ns=(when_ms * 1_000_000, when_ms * 1_000_000))


def port_is_open(port: int) -> bool:
    with socket.socket() as probe:
        probe.settimeout(0.2)
        return probe.connect_ex(("127.0.0.1", port)) == 0


def drivers_on(ports: list[int]) -> list[int]:
    """Driver processes left behind by an earlier run, by the port they hold.

    `tauri-driver` starts `WebKitWebDriver` itself and does not take it down
    with it, so an interrupted run leaves the native half listening and the
    next run dies on "Unable to listen". Matching on the port keeps this to
    processes this suite started.
    """
    wanted = {str(port) for port in ports}
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            command = entry.joinpath("cmdline").read_bytes().decode(errors="replace")
        except OSError:
            continue
        parts = [part for part in command.split("\0") if part]
        if not parts:
            continue
        name = Path(parts[0]).name
        if name not in {"tauri-driver", "WebKitWebDriver"}:
            continue
        if any(any(port in part for port in wanted) for part in parts[1:]):
            found.append(int(entry.name))
    return found


def processes_for(data_dir: Path) -> list[int]:
    """Every running app process configured with this data directory.

    Both instances run the same binary from the same path with no arguments, so
    the command line cannot tell them apart. The environment can.
    """
    wanted = f"OWL_DATA_DIR={data_dir}".encode()
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.joinpath("exe").resolve() != BINARY.resolve():
                continue
            environ = entry.joinpath("environ").read_bytes()
        except (OSError, RuntimeError):
            continue
        if wanted + b"\0" in environ:
            found.append(int(entry.name))
    return found


class Instance:
    """One running copy of the app, with the folder and the driver behind it."""

    def __init__(
        self,
        tag: str,
        name: str,
        root: Path,
        tcp_port: int,
        beacon_port: int,
        driver_port: int,
        native_port: int,
    ) -> None:
        self.tag = tag
        self.name = name
        self.tcp_port = tcp_port
        self.beacon_port = beacon_port
        self.driver_port = driver_port
        self.native_port = native_port

        self.home = root / tag
        self.data = self.home / "data"
        self.folder = self.home / "folder"
        self.script = self.home / "run.sh"
        self.log = self.home / "app.log"
        self.driver_log = self.home / "driver.log"

        self.data.mkdir(parents=True, exist_ok=True)
        self.folder.mkdir(parents=True, exist_ok=True)
        # The device name belongs to the engine, which reads it out of this
        # file. Writing it here rather than typing it into Settings is what
        # gives the two instances names a test can tell apart from the start.
        (self.data / "settings.json").write_text(
            json.dumps({"folder": str(self.folder), "device_name": name}, indent=2)
        )
        self._write_script()

        self.driver_process: subprocess.Popen | None = None
        self.driver: webdriver.Remote | None = None
        # The device id, read once the first session is up. Peers are matched
        # on it rather than on the name, which a test changes and puts back.
        self.device_id: str | None = None

    # Starting, stopping and restarting

    def _write_script(self) -> None:
        self.script.write_text(
            "#!/bin/sh\n"
            "# Written by the end to end suite: this instance's environment,\n"
            "# then the app. tauri-driver is pointed at this rather than at the\n"
            "# binary, because that is the only way to give each instance an\n"
            "# environment of its own.\n"
            f"export OWL_DATA_DIR={self.data}\n"
            f"export OWL_FOLDER={self.folder}\n"
            f"export OWL_PORT={self.tcp_port}\n"
            f"export OWL_BEACON_PORT={self.beacon_port}\n"
            "export GDK_BACKEND=x11\n"
            f'exec {BINARY} "$@" >>{self.log} 2>&1\n'
        )
        self.script.chmod(0o755)

    def start(self) -> None:
        self._start_driver()
        self.open_session()

    def _start_driver(self) -> None:
        if self.driver_process is not None and self.driver_process.poll() is None:
            return
        handle = self.driver_log.open("ab")
        self.driver_process = subprocess.Popen(
            [
                str(TAURI_DRIVER),
                "--port",
                str(self.driver_port),
                "--native-port",
                str(self.native_port),
            ],
            stdout=handle,
            stderr=subprocess.STDOUT,
            # Its own process group, so that stopping it stops the
            # WebKitWebDriver it started as well. Left alone, that one keeps
            # its port and the next run cannot bind it.
            start_new_session=True,
        )
        def listening() -> bool:
            if self.driver_process is not None and self.driver_process.poll() is not None:
                raise AssertionError(
                    f"tauri-driver for {self.tag} exited at once. Port {self.driver_port} or "
                    f"{self.native_port} is most likely still held by an earlier run: "
                    f"see {self.driver_log}."
                )
            return port_is_open(self.driver_port)

        wait_until(
            listening,
            20.0,
            f"tauri-driver for {self.tag} did not listen on {self.driver_port}",
            interval=0.1,
        )

    def open_session(self) -> None:
        options = webdriver.ChromeOptions()
        options.set_capability("browserName", "wry")
        options.set_capability("tauri:options", {"application": str(self.script)})
        self.driver = webdriver.Remote(f"http://127.0.0.1:{self.driver_port}", options=options)
        self.driver.set_script_timeout(60)
        wait_until(self._is_up, STARTUP, f"{self.tag} did not finish starting")
        self.device_id = self.state()["device"]["id"]

    def _is_up(self) -> bool:
        try:
            state = self.state()
        except Exception:
            return False
        # A port already taken is the usual reason, and waiting out the whole
        # timeout for it hides the one line that says so.
        for message in state.get("errors", []):
            if message.startswith("Owl Transfer could not start"):
                raise AssertionError(f"{self.tag}: {message}")
        return bool(state["device"]["id"]) and self.find(".app") is not None

    def close_session(self) -> None:
        """Ends the session, which takes the app process down with it."""
        if self.driver is None:
            return
        try:
            self.driver.quit()
        except Exception:
            pass
        self.driver = None
        wait_until(
            lambda: not processes_for(self.data),
            15.0,
            f"{self.tag} was still running after its session ended",
            interval=0.1,
        )

    def restart(self) -> float:
        """Stops the app and starts it again. Returns how long the start took."""
        self.close_session()
        started = time.monotonic()
        self.open_session()
        return time.monotonic() - started

    def stop(self) -> None:
        self.close_session()
        if self.driver_process is not None:
            group = os.getpgid(self.driver_process.pid)
            os.killpg(group, signal.SIGTERM)
            try:
                self.driver_process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(group, signal.SIGKILL)
                self.driver_process.wait(timeout=10)
            wait_until(
                lambda: not drivers_on([self.driver_port, self.native_port]),
                10.0,
                f"the drivers for {self.tag} did not stop",
                interval=0.1,
            )
            self.driver_process = None

    def set_beacon_port(self, port: int) -> None:
        """For the one step that puts both instances on the same beacon port."""
        self.beacon_port = port
        self._write_script()

    # The engine, through the page

    def invoke(self, command: str, **args):
        """One command, the way the interface sends it.

        WebDriver resolves a promise returned from a script before it answers,
        which is what makes this a plain call rather than an asynchronous one.
        """
        return self.driver.execute_script(
            "return window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1])",
            command,
            args,
        )

    def state(self) -> dict:
        return self.invoke("get_state")

    def list_dir(self, rel: str = "") -> list[dict]:
        return self.invoke("list_dir", path=rel)

    def peers(self) -> list[dict]:
        return self.state()["peers"]

    def connected_to_id(self, device_id: str) -> bool:
        return any(p["id"] == device_id and p["connected"] for p in self.peers())

    def connected_to(self, other: "Instance") -> bool:
        return self.connected_to_id(other.device_id or "")

    def transfers_running(self) -> int:
        return len(self.state()["transfers"]["active"])

    def entry(self, rel: str, name: str) -> dict | None:
        for item in self.list_dir(rel):
            if item["name"] == name:
                return item
        return None

    # The screen

    def elements(self, css: str) -> list[WebElement]:
        return self.driver.find_elements(By.CSS_SELECTOR, css)

    @staticmethod
    def text_of(element: WebElement) -> str:
        return (element.get_attribute("textContent") or "").strip()

    def find(self, css: str, text: str | None = None) -> WebElement | None:
        for element in self.elements(css):
            if text is None or self.text_of(element) == text:
                return element
        return None

    def texts(self, css: str) -> list[str]:
        return [self.text_of(element) for element in self.elements(css)]

    def wait_for(self, css: str, text: str | None = None, timeout: float = UI) -> WebElement:
        said = css if text is None else f"{css} saying {text!r}"
        element, _ = wait_until(lambda: self.find(css, text), timeout, f"{self.tag}: no {said}")
        return element

    def click(self, css: str, text: str | None = None, timeout: float = UI) -> None:
        self.wait_for(css, text, timeout).click()

    def type_into(self, element: WebElement, text: str) -> None:
        """Replaces what is in a controlled input, through real key events.

        `clear()` sets the value behind React's back and the component puts the
        old one straight back; selecting everything and typing over it is what a
        person does and what React hears.

        The wait at the end is not ceremony. The keystrokes are delivered and
        answered one at a time, and a submit sent before the last of them has
        been through React presses a button the component still thinks should
        be disabled, which does nothing and looks like sync losing a file.
        """
        element.click()
        element.send_keys(Keys.CONTROL, "a")
        element.send_keys(text)
        wait_until(
            lambda: element.get_attribute("value") == text,
            UI,
            f"{self.tag}: the field did not take {text!r}",
        )

    def pane(self) -> str:
        return self.wait_for(".app").get_attribute("data-pane") or ""

    def go(self, pane: str) -> None:
        """Opens Files, Devices or Settings from the sidebar."""
        if self.pane() == pane.lower():
            return
        self.click(".nav-item", pane)
        wait_until(lambda: self.pane() == pane.lower(), UI, f"{self.tag}: {pane} did not open")

    def status(self) -> str:
        found = self.find(".status-text")
        return "" if found is None else self.text_of(found)

    def field(self, label: str) -> WebElement:
        """The input a visible label names, found through the label's `for`."""
        found = None
        for element in self.elements("label"):
            if self.text_of(element) == label:
                found = element
                break
        assert found is not None, f"{self.tag}: no label {label!r}"
        return self.driver.find_element(By.ID, found.get_attribute("for"))

    def row(self, name: str) -> WebElement | None:
        """The file list row for one entry, by the name it shows."""
        for element in self.elements(".files-list .row"):
            shown = element.find_elements(By.CSS_SELECTOR, ".row-name")
            if shown and self.text_of(shown[0]) == name:
                return element
        return None

    def wait_for_row(self, name: str, timeout: float = UI) -> WebElement:
        element, _ = wait_until(
            lambda: self.row(name), timeout, f"{self.tag}: no row named {name!r}"
        )
        return element

    def go_root(self) -> None:
        """Back to the top of the folder, wherever the file list was left."""
        self.go("Files")
        first = self.find(".crumbs button.crumb")
        if first is None:
            return
        first.click()
        wait_until(
            lambda: self.find(".crumbs button.crumb") is None,
            UI,
            f"{self.tag}: the file list did not go back to the root",
        )

    def open_folder(self, name: str) -> None:
        self.wait_for_row(name).find_element(By.CSS_SELECTOR, ".row-main").click()
        wait_until(
            lambda: self.text_of(self.wait_for(".crumb-here")) == name,
            UI,
            f"{self.tag}: {name} did not open",
        )

    def open_row_menu(self, name: str) -> None:
        self.wait_for_row(name).find_element(
            By.CSS_SELECTOR, f'[aria-label="More for {name}"]'
        ).click()
        self.wait_for('[role="menu"]')

    def menu_item(self, label: str) -> None:
        self.click('[role="menuitem"]', label)

    def dialog_submit(self, label: str) -> None:
        self.click('[role="dialog"] button[type="submit"]', label)

    def screenshot(self, into: Path) -> None:
        into.parent.mkdir(parents=True, exist_ok=True)
        into.write_bytes(self.driver.get_screenshot_as_png())

    # Steps used by more than one test

    def pair_by_address(self, other: "Instance") -> None:
        self.go("Devices")
        self.type_into(self.field("Address"), "127.0.0.1")
        self.type_into(self.field("Port"), str(other.tcp_port))
        self.click("button", "Connect")

    def accept_pairing(self) -> None:
        self.go("Devices")
        self.click(".pairing-actions button", "Accept")

    def forget(self, other: "Instance") -> None:
        """Presses Forget on one paired device and confirms the dialog."""
        self.go("Devices")
        for element in self.elements(".devices-list .device"):
            shown = element.find_elements(By.CSS_SELECTOR, ".device-name")
            if shown and self.text_of(shown[0]) == other.name:
                element.find_element(By.CSS_SELECTOR, "button").click()
                self.dialog_submit("Forget")
                return
        raise AssertionError(f"{self.tag}: {other.name} is not in the paired list")
