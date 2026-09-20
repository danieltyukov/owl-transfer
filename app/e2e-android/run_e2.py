"""Task E2: one Linux desktop and one Android emulator, end to end.

Run it with `./run.sh`. It starts a desktop instance of its own, installs the
debug APK on the emulator, walks the storage permission card, pairs the two by
address, and then does everything task E2 asks for in both directions,
recording what each step took. It prints a table at the end.

The desktop side is the same `Instance` the desktop suite uses. The phone side
is `phone.Phone`: adb for the device and the folder, the debug WebView over CDP
for the interface, and `uiautomator` for the one screen that is not this app's.

This is a run rather than a suite. It leaves the emulator as it found it:
the app uninstalled, the sync folder gone and the pushed picture removed.
"""

from __future__ import annotations

import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "e2e-desktop"))

from owl import Instance, drivers_on, processes_for  # noqa: E402
from owl import wait_until as desktop_wait  # noqa: E402
from phone import PACKAGE, Phone, wait_until  # noqa: E402

REPO = HERE.parents[1]
APK = (
    REPO
    / "app/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"
)
SERIAL = os.environ.get("OWL_E2E_SERIAL", "emulator-5556")
WORK = Path(os.environ.get("OWL_E2E_WORK", "/tmp/owl-e2e-android"))
# The address of the host from inside the emulator, which is what makes pair by
# address the only way in: the emulator is behind a NAT and hears no broadcast.
HOST = "10.0.2.2"
MIB = 1024 * 1024

STEPS: list[tuple[str, str, str, str]] = []


def record(step: str, result: str, seconds: float, note: str = "") -> None:
    STEPS.append((step, result, f"{seconds:.1f} s", note))
    print(f"  {result:<7} {step} ({seconds:.1f} s) {note}", flush=True)


def table() -> str:
    rows = ["| Step | Result | Time | Measured |", "| --- | --- | --- | --- |"]
    for step, result, took, note in STEPS:
        rows.append(f"| {step} | {result} | {took} | {note} |")
    return "\n".join(rows)


class Step:
    """One step, timed, recorded whether it passed or not."""

    def __init__(self, name: str) -> None:
        self.name = name
        self.note = ""

    def __enter__(self) -> "Step":
        print(f"- {self.name}", flush=True)
        self.started = time.monotonic()
        return self

    def says(self, note: str) -> None:
        self.note = f"{self.note}; {note}" if self.note else note

    def __exit__(self, kind, value, trace) -> bool:
        """Records what happened and lets a failure through.

        Every step after a failed one stands on what it was supposed to leave
        behind, so carrying on turns one honest failure into a column of
        meaningless ones and a quarter of an hour of timeouts.
        """
        took = time.monotonic() - self.started
        if kind is None:
            record(self.name, "passed", took, self.note)
        else:
            record(self.name, "FAILED", took, f"{self.note} [{value}]".strip("; "))
        return False


def main() -> int:
    assert APK.exists(), f"no APK at {APK}. Build one with npx tauri android build --debug"
    if WORK.exists():
        shutil.rmtree(WORK)
    WORK.mkdir(parents=True)

    for pid in drivers_on([4444, 4464]):
        os.kill(pid, signal.SIGKILL)

    desktop = Instance(
        tag="a",
        name="Owl desktop",
        root=WORK,
        tcp_port=52734,
        beacon_port=52735,
        driver_port=4444,
        native_port=4464,
    )
    for pid in processes_for(desktop.data):
        os.kill(pid, signal.SIGKILL)

    phone = Phone(SERIAL)
    try:
        run(desktop, phone)
    except Exception as trouble:
        print(f"the run stopped: {trouble}", flush=True)
        try:
            phone.screencap(WORK / "where-it-stopped.png")
            print(f"the screen at that moment: {WORK / 'where-it-stopped.png'}", flush=True)
        except Exception:
            pass
    finally:
        desktop.stop()
        clean_up(phone)

    print()
    print(table())
    Path(os.environ.get("OWL_E2E_REPORT", WORK / "table.md")).write_text(table() + "\n")
    return 1 if any(row[1] != "passed" for row in STEPS) else 0


def run(desktop: Instance, phone: Phone) -> None:
    with Step("the desktop instance starts") as step:
        desktop.start()
        step.says(f"folder {desktop.folder}")

    with Step("a fresh install shows the permission card") as step:
        phone.uninstall()
        phone.install(APK)
        phone.launch()
        phone.attach()
        assert phone.state()["paused"], "the engine did not start paused"
        phone.tab("Settings")
        card, _ = wait_until(lambda: storage_card(phone), 20.0, "the storage card never appeared")
        assert "Not allowed" in card, card
        assert phone.storage_permission().strip().startswith("No operations")

    with Step("the card leads to the system screen and the grant resumes the engine") as step:
        assert phone.press(".panel button", "Open Android settings")
        phone.tap_by_text("Allow access to manage all files")
        wait_until(
            lambda: "allow" in phone.storage_permission(), 20.0, "the grant did not take"
        )
        phone.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        phone.attach()
        _, waited = wait_until(
            lambda: not phone.state()["paused"], 30.0, "the engine did not resume"
        )
        step.says(f"resumed {waited:.1f} s after coming back, with nothing tapped")
        wait_until(lambda: "Allowed" in storage_card(phone), 10.0, "the card did not say Allowed")
        wait_until(
            lambda: phone.shell(f"[ -d {phone.FOLDER} ] && echo yes").strip() == "yes",
            20.0,
            "the sync folder was never created",
        )

    with Step("the phone pairs with the desktop by address") as step:
        phone.tab("Devices")
        assert phone.fill("input[placeholder='192.168.1.31']", HOST)
        assert phone.press("button", "Connect")
        _, waited = wait_until(
            lambda: desktop.state()["pendingPairing"] and phone.state()["pendingPairing"],
            30.0,
            "the pairing request did not reach both",
        )
        here = desktop.state()["pendingPairing"]["code"]
        there = phone.state()["pendingPairing"]["code"]
        assert here == there, f"the codes differ: {here} and {there}"
        step.says(f"code {here} on both after {waited:.1f} s")

        desktop.accept_pairing()
        _, waited = wait_until(
            lambda: desktop.connected_to_id(phone.state()["device"]["id"])
            and phone.connected_to(desktop.device_id),
            15.0,
            "the two did not connect",
        )
        step.says(f"connected {waited:.1f} s after Accept")
        assert any(p["kind"] == "phone" for p in desktop.peers()), "the desktop calls it a desktop"

    with Step("a file dropped into the desktop folder reaches the phone") as step:
        content = os.urandom(4096)
        (desktop.folder / "from-desktop.bin").write_bytes(content)
        _, waited = wait_until(
            lambda: phone.size("from-desktop.bin") == len(content),
            30.0,
            "the file did not reach the phone",
        )
        step.says(f"4 KiB in {waited:.1f} s")
        assert phone.read("from-desktop.bin") == content

    with Step("a photo shared to the app from the gallery reaches the desktop") as step:
        picture = WORK / "kestrel.png"
        picture.write_bytes(png_bytes())
        phone.adb("push", str(picture), "/sdcard/Pictures/kestrel.png")
        phone.shell(
            "am broadcast -a android.intent.action.MEDIA_SCANNER_SCAN_FILE "
            "-d file:///sdcard/Pictures/kestrel.png"
        )
        uri, _ = wait_until(lambda: media_uri(phone, "kestrel.png"), 30.0, "MediaStore never saw it")
        step.says(f"shared {uri}")
        started = time.monotonic()
        phone.shell(
            "am start -a android.intent.action.SEND -t image/png "
            f"--eu android.intent.extra.STREAM {uri} -n {PACKAGE}/.MainActivity"
        )
        _, waited = wait_until(
            lambda: (desktop.folder / "kestrel.png").exists(),
            60.0,
            "the shared photo did not reach the desktop",
        )
        assert (desktop.folder / "kestrel.png").read_bytes() == picture.read_bytes()
        step.says(f"kept its name, on the desktop {time.monotonic() - started:.1f} s after the share")
        phone.attach()

    with Step("an edit on the phone reaches the desktop") as step:
        phone.shell("echo 'edited on the phone' > /sdcard/OwlTransfer/from-desktop.bin")
        _, waited = wait_until(
            lambda: (desktop.folder / "from-desktop.bin").read_bytes()
            == b"edited on the phone\n",
            30.0,
            "the edit did not reach the desktop",
        )
        step.says(f"{waited:.1f} s, which includes the two second poll on shared storage")

    with Step("an edit on the desktop reaches the phone") as step:
        (desktop.folder / "from-desktop.bin").write_bytes(b"edited on the desktop\n")
        _, waited = wait_until(
            lambda: phone.read("from-desktop.bin") == b"edited on the desktop\n",
            30.0,
            "the edit did not reach the phone",
        )
        step.says(f"{waited:.1f} s")

    with Step("a rename through the phone's menu reaches the desktop") as step:
        phone.tab("Files")
        wait_until(
            lambda: phone.press('[aria-label="More for from-desktop.bin"]'),
            20.0,
            "the row never appeared on the phone",
        )
        wait_until(lambda: phone.press('[role="menuitem"]', "Rename"), 10.0, "no Rename item")
        assert phone.fill('[role="dialog"] input[aria-label="Name"]', "notes.txt")
        assert phone.press('[role="dialog"] button[type="submit"]', "Rename")
        _, waited = wait_until(
            lambda: (desktop.folder / "notes.txt").exists()
            and not (desktop.folder / "from-desktop.bin").exists(),
            30.0,
            "the rename did not reach the desktop",
        )
        step.says(f"{waited:.1f} s")

    with Step("a delete through the phone's menu reaches the desktop") as step:
        wait_until(
            lambda: phone.press('[aria-label="More for notes.txt"]'), 20.0, "no row for notes.txt"
        )
        wait_until(lambda: phone.press('[role="menuitem"]', "Delete"), 10.0, "no Delete item")
        assert phone.press('[role="dialog"] button[type="submit"]', "Delete")
        _, waited = wait_until(
            lambda: not (desktop.folder / "notes.txt").exists(),
            30.0,
            "the delete did not reach the desktop",
        )
        step.says(f"{waited:.1f} s")

    with Step("a folder made on the desktop reaches the phone") as step:
        (desktop.folder / "Trip" / "Day one").mkdir(parents=True)
        (desktop.folder / "Trip" / "Day one" / "note.txt").write_bytes(b"in a folder\n")
        _, waited = wait_until(
            lambda: phone.read("Trip/Day one/note.txt") == b"in a folder\n",
            30.0,
            "the folder did not reach the phone",
        )
        step.says(f"{waited:.1f} s")

    with Step("twenty megabytes from the desktop to the phone") as step:
        content = os.urandom(20 * MIB)
        (desktop.folder / "to-phone.bin").write_bytes(content)
        _, waited = wait_until(
            lambda: phone.size("to-phone.bin") == len(content),
            300.0,
            "the 20 MB file did not reach the phone",
            interval=0.25,
        )
        step.says(f"{waited:.1f} s, {20 / waited:.1f} MiB/s")
        import hashlib

        assert phone.sha("to-phone.bin") == hashlib.sha256(content).hexdigest()

    with Step("twenty megabytes from the phone to the desktop") as step:
        phone.shell(
            f"dd if=/dev/urandom of={phone.FOLDER}/to-desktop.bin bs=1048576 count=20",
            timeout=300.0,
        )
        target = desktop.folder / "to-desktop.bin"
        _, waited = wait_until(
            lambda: target.exists() and target.stat().st_size == 20 * MIB,
            300.0,
            "the 20 MB file did not reach the desktop",
            interval=0.25,
        )
        import hashlib

        here = hashlib.sha256(target.read_bytes()).hexdigest()
        assert here == phone.sha("to-desktop.bin"), "the bytes differ"
        step.says(f"{waited:.1f} s, {20 / waited:.1f} MiB/s")

    with Step("the phone is killed, comes back and catches up") as step:
        phone.force_stop()
        desktop_wait(
            lambda: not any(p["connected"] for p in desktop.peers()),
            30.0,
            "the desktop did not notice the phone had gone",
        )
        away = b"made while the phone was away\n"
        (desktop.folder / "while-away.txt").write_bytes(away)

        back = time.monotonic()
        phone.launch()
        phone.attach()
        _, waited = wait_until(
            lambda: phone.connected_to(desktop.device_id), 60.0, "the phone did not reconnect"
        )
        step.says(f"reconnected {waited:.1f} s after the launch")
        _, caught = wait_until(
            lambda: phone.read("while-away.txt") == away, 60.0, "the phone did not catch up"
        )
        step.says(f"caught up {time.monotonic() - back:.1f} s after the launch")

    with Step("a file made while the phone is on the home screen") as step:
        phone.home()
        time.sleep(30)
        backgrounded = phone.connected_to(desktop.device_id)
        step.says(f"still connected after 30 s on the home screen: {backgrounded}")

        content = b"made while the phone was on the home screen\n"
        made = time.monotonic()
        (desktop.folder / "while-home.txt").write_bytes(content)
        time.sleep(10)
        in_background = phone.read("while-home.txt") == content
        step.says(f"arrived without the app on screen: {in_background}")

        phone.launch()
        phone.attach()
        _, waited = wait_until(
            lambda: phone.read("while-home.txt") == content,
            60.0,
            "the file never arrived, even on resume",
        )
        step.says(f"on the phone {time.monotonic() - made:.1f} s after it was made")


def storage_card(phone: Phone) -> str:
    """The words on the Storage access panel, which has no id of its own."""
    return (
        phone.evaluate(
            "(() => { const c = [...document.querySelectorAll('.panel')]"
            ".find(c => c.textContent.includes('Storage access'));"
            " return c ? c.textContent.trim() : ''; })()"
        )
        or ""
    )


def media_uri(phone: Phone, name: str) -> str | None:
    """The content URI MediaStore gave the pushed picture."""
    out = phone.shell(
        "content query --uri content://media/external/images/media "
        "--projection _id:_display_name"
    )
    for line in out.splitlines():
        if name in line and "_id=" in line:
            number = line.split("_id=")[1].split(",")[0].strip()
            return f"content://media/external/images/media/{number}"
    return None


def png_bytes() -> bytes:
    """A small real PNG, so the share carries an image rather than a name."""
    import struct
    import zlib

    width = height = 64
    rows = b""
    for y in range(height):
        rows += b"\0" + bytes(
            value
            for x in range(width)
            for value in (229, 165 if (x + y) % 16 < 8 else 90, 74)
        )

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(rows))
        + chunk(b"IEND", b"")
    )


def clean_up(phone: Phone) -> None:
    """Leaves the emulator as it was found."""
    try:
        phone.detach()
        phone.shell(f"rm -rf {phone.FOLDER}")
        phone.shell("rm -f /sdcard/Pictures/kestrel.png")
        phone.shell(
            "content delete --uri content://media/external/images/media "
            "--where \"_display_name='kestrel.png'\""
        )
        phone.uninstall()
        subprocess.run(
            ["adb", "-s", phone.serial, "shell", "appops", "set", PACKAGE,
             "MANAGE_EXTERNAL_STORAGE", "default"],
            capture_output=True,
        )
    except Exception as trouble:
        print(f"could not finish cleaning up: {trouble}")


if __name__ == "__main__":
    raise SystemExit(main())
