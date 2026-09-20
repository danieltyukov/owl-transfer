"""Two Owl Transfer desktops on one Linux machine, end to end.

The steps are the ones in the plan's task E1, in order, and they share one pair
of running instances: A on 52734 and B on 52744, with beacons on ports that
cannot hear each other, so B pairs by typing A's address. Each test is one
step, each waits with a timeout rather than a sleep, and what a step measured
goes into the table the run prints at the end.

Two tests are marked `xfail(strict=True)`. They are the two places where the
app does less than task E1 asks for, both of them in the engine, and the mark
is how the suite records that without going red every run. If either of them
starts passing, the run fails and whoever fixed the engine gets to delete the
mark.
"""

from __future__ import annotations

import hashlib
import os
import time
from pathlib import Path

import pytest
from selenium.webdriver.common.keys import Keys

from owl import BIG_SYNC, SYNC, UI, mtime_ms, set_mtime_ms, wait_until

KIB = 1024
MIB = 1024 * 1024


def digest(path: Path) -> str:
    reader = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            reader.update(block)
    return reader.hexdigest()


def has_bytes(path: Path, expected: bytes) -> bool:
    """True once the file is there and holds exactly these bytes.

    The engine writes into `.owl-tmp-<hash>` and renames, so a half written
    file is never under the real name; the read is still guarded, because the
    listing and the rename race on a folder being watched.
    """
    try:
        return path.read_bytes() == expected
    except OSError:
        return False


def conflict_copies(folder: Path, name: str) -> list[str]:
    stem, _, ext = name.rpartition(".")
    return sorted(
        item.name
        for item in folder.iterdir()
        if item.name.startswith(f"{stem} (conflict from ") and item.name.endswith(f".{ext}")
    )


def test_01_pairing(pair, world, measure):
    """B pairs with A by address, the codes match, and A accepts."""
    a, b = pair

    assert a.status() == "No device paired"
    assert b.status() == "No device paired"
    world["status_seen"].add("No device paired")
    assert a.state()["peers"] == []

    asked = time.monotonic()
    b.pair_by_address(a)

    def both_asking():
        one, two = a.state()["pendingPairing"], b.state()["pendingPairing"]
        return None if one is None or two is None else (one, two)

    (incoming, outgoing), waited = wait_until(
        both_asking, UI, "the pairing request did not reach both sides"
    )
    measure(f"request on both screens in {waited:.1f} s")

    assert incoming["code"] == outgoing["code"], "the two screens showed different codes"
    assert len(incoming["code"].replace(" ", "")) == 6
    assert incoming["direction"] == "incoming"
    assert outgoing["direction"] == "outgoing"

    a.go("Devices")
    card = a.wait_for(".pairing-code")
    assert a.text_of(card) == incoming["code"]
    # Read out as digits rather than as a number, which is the only way to
    # check one screen against the other.
    spelled = ", ".join(" ".join(group) for group in incoming["code"].split())
    assert card.get_attribute("aria-label") == spelled
    assert a.text_of(a.find(".pairing-lead")) == f"{b.name} wants to pair"
    assert b.text_of(b.find(".pairing-lead")) == f"Pairing with {a.name}"
    assert b.text_of(b.find(".pairing-code")) == outgoing["code"]

    accepted = time.monotonic()
    a.click(".pairing-actions button", "Accept")
    _, waited = wait_until(
        lambda: a.connected_to(b) and b.connected_to(a),
        5.0,
        "the two did not list each other as connected",
    )
    measure(f"connected {waited:.1f} s after Accept, request to connected {time.monotonic() - asked:.1f} s")

    # The screen follows the engine by an event, so it is waited for rather
    # than read the instant the state says the two are connected.
    a.wait_for(".devices-list .device-dot[data-connected]")
    b.wait_for(".devices-list .device-dot[data-connected]")
    wait_until(
        lambda: b.name in a.texts(".devices-list .device-name"), UI, "A did not list B as paired"
    )
    wait_until(
        lambda: a.name in b.texts(".devices-list .device-name"), UI, "B did not list A as paired"
    )
    assert a.state()["pendingPairing"] is None


def test_02_a_new_file_lands_on_the_other_side(pair, world, measure):
    """One kibibyte written into A's folder from outside the app."""
    a, b = pair
    content = os.urandom(KIB)

    written = time.monotonic()
    (a.folder / "hello.bin").write_bytes(content)
    _, waited = wait_until(
        lambda: has_bytes(b.folder / "hello.bin", content), SYNC, "hello.bin did not reach B"
    )
    measure(f"1 KiB on disk in {waited * 1000:.0f} ms")

    assert mtime_ms(b.folder / "hello.bin") == mtime_ms(a.folder / "hello.bin")

    _, shown = wait_until(
        lambda: b.entry("", "hello.bin"), UI, "hello.bin never appeared in B's list"
    )
    measure(f"in the list {time.monotonic() - written:.1f} s after the write")

    _, _ = wait_until(
        lambda: a.status() == "Up to date" and b.status() == "Up to date",
        UI,
        "the status strip did not settle on Up to date",
    )
    world["status_seen"].add("Up to date")


def test_03_an_edit_comes_back(pair, measure):
    """The same file changed on B reaches A."""
    a, b = pair
    content = b"the second version, written on B\n" + os.urandom(64)

    (b.folder / "hello.bin").write_bytes(content)
    _, waited = wait_until(
        lambda: has_bytes(a.folder / "hello.bin", content), SYNC, "the edit did not reach A"
    )
    measure(f"edit back in {waited * 1000:.0f} ms")
    assert mtime_ms(a.folder / "hello.bin") == mtime_ms(b.folder / "hello.bin")


def test_04_a_rename_moves_no_bytes(pair, measure):
    """Renamed through A's menu. B has the same file under the new name.

    The engine looks for a local file with the same hash before it asks the
    network for one, so a rename on one side is a local copy on the other and
    no transfer is started. What is asserted here is that nothing was
    transferred at any point while the new name was arriving.
    """
    a, b = pair
    content = (a.folder / "hello.bin").read_bytes()

    a.go_root()
    a.wait_for_row("hello.bin")
    a.open_row_menu("hello.bin")
    a.menu_item("Rename")
    dialog = a.wait_for('[role="dialog"]')
    assert a.text_of(dialog.find_element("css selector", ".dialog-title")) == "Rename hello.bin"
    a.type_into(a.wait_for('[role="dialog"] input[aria-label="Name"]'), "greeting.bin")
    a.dialog_submit("Rename")

    busiest = 0

    def landed():
        nonlocal busiest
        busiest = max(busiest, a.transfers_running(), b.transfers_running())
        return has_bytes(b.folder / "greeting.bin", content)

    _, waited = wait_until(landed, SYNC, "the new name did not reach B")
    measure(f"new name in {waited * 1000:.0f} ms, transfers seen {busiest}")

    assert busiest == 0, "a rename started a transfer"
    assert not (b.folder / "hello.bin").exists()
    assert not (a.folder / "hello.bin").exists()
    assert a.entry("", "greeting.bin") is not None
    _, _ = wait_until(lambda: b.entry("", "greeting.bin"), UI, "B's list kept the old name")


def test_05_a_folder_tree_of_ten_files(pair, measure):
    """A folder with two subfolders and ten files, created from outside."""
    a, b = pair
    wanted = {}
    for group in ("photos", "notes"):
        (a.folder / "trip" / group).mkdir(parents=True, exist_ok=True)
        for index in range(1, 6):
            rel = f"trip/{group}/{group[:-1]}-{index}.bin"
            wanted[rel] = os.urandom(4 * KIB)

    started = time.monotonic()
    for rel, content in wanted.items():
        (a.folder / rel).write_bytes(content)

    _, waited = wait_until(
        lambda: all(has_bytes(b.folder / rel, content) for rel, content in wanted.items()),
        SYNC,
        "not every file in the tree reached B",
    )
    measure(f"ten files in {waited:.1f} s")
    assert time.monotonic() - started < SYNC

    assert sorted(item["name"] for item in b.list_dir("trip")) == ["notes", "photos"]
    assert len(b.list_dir("trip/photos")) == 5


def test_06_a_delete_from_the_interface_propagates(pair, measure):
    """One file deleted on B through the menu goes from A as well."""
    a, b = pair

    b.go_root()
    b.open_folder("trip")
    b.open_folder("photos")
    b.wait_for_row("photo-3.bin")

    b.open_row_menu("photo-3.bin")
    b.menu_item("Delete")
    dialog = b.wait_for('[role="dialog"]')
    assert b.text_of(dialog.find_element("css selector", ".dialog-title")) == "Delete photo-3.bin?"
    assert b.text_of(dialog.find_element("css selector", ".dialog-text")) == "It goes from both devices."
    b.dialog_submit("Delete")

    _, waited = wait_until(
        lambda: not (a.folder / "trip/photos/photo-3.bin").exists(),
        SYNC,
        "the delete did not reach A",
    )
    measure(f"delete in {waited * 1000:.0f} ms")
    assert not (b.folder / "trip/photos/photo-3.bin").exists()
    assert len(a.list_dir("trip/photos")) == 4


def test_07_a_fifty_megabyte_file(pair, measure):
    """Fifty megabytes of random bytes, with the rate it went at."""
    a, b = pair
    content = os.urandom(50 * MIB)
    source = a.folder / "big.bin"
    target = b.folder / "big.bin"

    started = time.monotonic()
    source.write_bytes(content)
    _, waited = wait_until(
        lambda: target.exists() and target.stat().st_size == len(content),
        BIG_SYNC,
        "the 50 MB file did not reach B",
        interval=0.01,
    )
    # The size is right the instant the rename lands; the hash is the proof.
    assert digest(target) == digest(source)
    assert mtime_ms(target) == mtime_ms(source)
    measure(f"50 MiB write to landed in {waited:.2f} s, {50 / waited:.0f} MiB/s")
    assert time.monotonic() - started < BIG_SYNC

    _, _ = wait_until(
        lambda: a.status() == "Up to date" and b.status() == "Up to date",
        UI,
        "the strip did not go back to Up to date",
    )


def test_08_a_conflict_keeps_both_versions(pair, world, measure):
    """B is killed, both sides edit the same file, B comes back."""
    a, b = pair
    first = b"the version both sides start from\n"
    (a.folder / "journal.txt").write_bytes(first)
    wait_until(
        lambda: has_bytes(b.folder / "journal.txt", first), SYNC, "journal.txt did not reach B"
    )

    b.close_session()

    losing = b"written on B while it was away\n"
    winning = b"written on A, and newer\n"
    (b.folder / "journal.txt").write_bytes(losing)
    (a.folder / "journal.txt").write_bytes(winning)
    # The newer mtime wins, so the two are set rather than raced for.
    now = int(time.time() * 1000)
    set_mtime_ms(b.folder / "journal.txt", now - 60_000)
    set_mtime_ms(a.folder / "journal.txt", now)

    came_back = time.monotonic()
    b.open_session()

    def settled():
        here, there = conflict_copies(a.folder, "journal.txt"), conflict_copies(b.folder, "journal.txt")
        return here == there and len(here) == 1 and here or None

    copies, waited = wait_until(settled, SYNC, "the two sides did not agree on one conflict copy")
    measure(f"resolved {waited:.1f} s after B came back")

    assert (a.folder / "journal.txt").read_bytes() == winning, "the newer edit lost its name"
    assert (b.folder / "journal.txt").read_bytes() == winning
    assert (a.folder / copies[0]).read_bytes() == losing
    assert (b.folder / copies[0]).read_bytes() == losing
    world["conflict_copy"] = copies[0]
    measure(f"copy named {copies[0]!r}")

    _, _ = wait_until(
        lambda: a.connected_to(b) and b.connected_to(a),
        15.0,
        "the two did not reconnect after the restart",
    )
    measure(f"B usable again {time.monotonic() - came_back:.1f} s after the kill")


def test_09_forget_on_the_dialling_side(pair, measure):
    """B, which dialled A, forgets it. B's own side goes quiet at once."""
    a, b = pair
    b.forget(a)
    _, waited = wait_until(lambda: b.state()["peers"] == [], UI, "B kept A in its list")
    measure(f"gone from B in {waited * 1000:.0f} ms")
    wait_until(lambda: b.status() == "No device paired", UI, "B's strip still had a device")
    assert b.texts(".devices-list .device-name") == []


@pytest.mark.xfail(
    strict=True,
    reason=(
        "the engine tells the other side by refusing its next connection, and "
        "the side that was dialled never dials: it stores no address for a "
        "peer that came to it. See the task E report, engine finding 1."
    ),
)
def test_09_the_side_that_was_dialled_is_told(pair):
    """A should hear that B forgot it, and forget B in turn."""
    a, _ = pair
    wait_until(lambda: a.state()["peers"] == [], 20.0, "A was never told")


def test_09_pairing_again_works(pair, measure):
    """With A's stale record cleared by hand, the two pair again."""
    a, b = pair
    if a.state()["peers"]:
        a.forget(b)
        wait_until(lambda: a.state()["peers"] == [], UI, "A kept B in its list")

    started = time.monotonic()
    b.pair_by_address(a)
    wait_until(
        lambda: a.state()["pendingPairing"] and b.state()["pendingPairing"],
        UI,
        "the second pairing request did not arrive",
    )
    a.accept_pairing()
    _, waited = wait_until(
        lambda: a.connected_to(b) and b.connected_to(a), 5.0, "the second pairing did not connect"
    )
    measure(f"paired again in {time.monotonic() - started:.1f} s")


def test_09_forget_on_the_side_that_was_dialled(pair, measure):
    """A forgets B. B is redialling, is refused, and says so."""
    a, b = pair
    b.go_root()
    a.forget(b)
    wait_until(lambda: a.state()["peers"] == [], UI, "A kept B in its list")

    _, waited = wait_until(lambda: b.state()["peers"] == [], 30.0, "B was never told")
    measure(f"B told in {waited:.1f} s")

    errors = b.state()["errors"]
    assert errors, "B forgot A without saying why"
    assert "no longer trusts this device" in errors[-1]
    assert a.name in errors[-1]


def test_09_pairing_again_after_the_reject(pair, measure):
    """The two pair a third time, from clean records on both sides."""
    a, b = pair
    started = time.monotonic()
    b.pair_by_address(a)
    wait_until(
        lambda: a.state()["pendingPairing"] and b.state()["pendingPairing"],
        UI,
        "the third pairing request did not arrive",
    )
    a.accept_pairing()
    wait_until(
        lambda: a.connected_to(b) and b.connected_to(a), 5.0, "the third pairing did not connect"
    )
    measure(f"paired again in {time.monotonic() - started:.1f} s")
    # Nothing in the folder was disturbed by any of it.
    assert (b.folder / "greeting.bin").exists()
    assert (b.folder / "big.bin").exists()


def test_10_the_other_side_reconnects_after_a_restart(pair, world, measure):
    """A is restarted while B runs. B redials and catches up."""
    a, b = pair

    a.close_session()
    wait_until(lambda: not b.connected_to(a), UI, "B did not notice that A went away")
    wait_until(
        lambda: b.status() == "No device connected", UI, "B's strip did not say A had gone"
    )
    world["status_seen"].add("No device connected")

    b.go_root()
    b.wait_for_row("greeting.bin")
    assert "Only here" in b.texts(".files-list .pill"), "a file B cannot reach was not marked"
    world["saw_only_here"] = True

    # Something made while the other side is away, for the catch up below.
    away = b"made while A was away\n"
    (b.folder / "while-away.txt").write_bytes(away)

    back = time.monotonic()
    a.open_session()
    _, waited = wait_until(lambda: b.connected_to(a), 10.0, "B did not reconnect within 10 s")
    measure(f"reconnected {waited:.1f} s after A came up, {time.monotonic() - back:.1f} s from the restart")

    _, caught = wait_until(
        lambda: has_bytes(a.folder / "while-away.txt", away), SYNC, "A did not catch up"
    )
    measure(f"caught up in {caught:.1f} s")


def test_11_the_interface_caught_mid_transfer(pair, world, measure):
    """A file big enough for the screen to be caught while it is moving.

    Fifty megabytes cross the loopback in less time than a round trip to the
    window takes, so the step that measures the rate cannot also watch the
    screen: the engine reports the transfer for about sixty milliseconds. This
    one is sized so that both the badge and the strip can be seen. Set
    OWL_E2E_WATCH_MIB to make it larger on a machine where it is still tight.
    """
    a, b = pair
    size = int(os.environ.get("OWL_E2E_WATCH_MIB", "200"))
    content = os.urandom(size * MIB)
    source = a.folder / "watched.bin"
    target = b.folder / "watched.bin"
    seen = {"badge": False, "strip": False, "transfer": False}

    a.go_root()
    source.write_bytes(content)

    def landed():
        if not seen["transfer"]:
            seen["transfer"] = a.transfers_running() > 0
        if not seen["badge"]:
            seen["badge"] = a.find(".badge[title^='Syncing']") is not None
        if not seen["strip"]:
            seen["strip"] = a.status().startswith("Syncing") or b.status().startswith("Syncing")
        return target.exists() and target.stat().st_size == len(content)

    _, waited = wait_until(landed, BIG_SYNC, f"the {size} MiB file did not reach B", interval=0.0)
    measure(f"{size} MiB in {waited:.2f} s; transfer {seen['transfer']}, badge {seen['badge']}, strip {seen['strip']}")

    assert seen["transfer"], "the engine never reported a transfer"
    assert seen["badge"], "no row was ever badged as syncing"
    assert seen["strip"], "the strip never said Syncing"
    world["status_seen"].add("Syncing")
    world["saw_syncing_badge"] = True

    # Half a gigabyte between the two folders is not worth keeping for the
    # rest of the run.
    source.unlink()
    wait_until(lambda: not target.exists(), SYNC, "the big file was not removed from B")
    wait_until(
        lambda: a.status() == "Up to date" and b.status() == "Up to date",
        UI,
        "the strip did not settle after the big file went",
    )


def test_11_the_status_strip_said_each_phase(pair, world):
    """Every phase of the run put its own sentence on the strip."""
    a, b = pair
    wait_until(lambda: a.status() == "Up to date", UI, "the strip did not settle")
    world["status_seen"].add(a.status())

    assert {"No device paired", "No device connected", "Up to date"} <= world["status_seen"]
    assert "Syncing" in world["status_seen"], "no transfer was ever caught on the strip"
    assert a.text_of(a.find(".status-meta")).startswith("1 device")


def test_11_the_file_badges(pair, world):
    """Synced, conflict, syncing and only here, each seen at least once."""
    a, b = pair
    a.go_root()
    a.wait_for_row("greeting.bin")

    assert a.find(".badge-synced") is not None, "nothing was marked as synced"
    assert "Conflict" in a.texts(".files-list .pill"), "the conflict copy was not marked"
    assert a.find(".pill-conflict") is not None
    assert world.get("saw_syncing_badge"), "the syncing badge was never caught"
    assert world.get("saw_only_here"), "the only here pill was never caught"

    row = a.row(world["conflict_copy"])
    assert row is not None


def test_11_new_folder(pair, measure):
    """The New folder dialog makes a folder, and it reaches B."""
    a, b = pair
    a.go_root()
    a.click("button[aria-label='New folder']")
    assert a.text_of(a.wait_for(".dialog-title")) == "New folder"
    a.type_into(a.wait_for('[role="dialog"] input[aria-label="Name"]'), "Made here")
    a.dialog_submit("Create")

    _, waited = wait_until(
        lambda: (b.folder / "Made here").is_dir(), SYNC, "the new folder did not reach B"
    )
    measure(f"folder in {waited * 1000:.0f} ms")
    assert a.row("Made here") is not None


def test_11_a_name_a_file_cannot_have_is_refused(pair):
    """The dialog says why rather than letting the engine fail."""
    a, _ = pair
    a.go_root()
    a.click("button[aria-label='New folder']")
    a.type_into(a.wait_for('[role="dialog"] input[aria-label="Name"]'), "one/two")
    assert a.text_of(a.wait_for(".dialog-text")) == "A name cannot contain a slash."
    assert a.wait_for('[role="dialog"] button[type="submit"]').get_attribute("disabled")
    a.click('[role="dialog"] button', "Cancel")
    wait_until(lambda: a.find('[role="dialog"]') is None, UI, "the dialog stayed open")


def test_11_the_device_name_changes(pair, measure):
    """Settings renames this device, and the engine keeps it."""
    _, b = pair
    b.go("Settings")
    field = b.wait_for("input[aria-label='Device name']")
    b.type_into(field, "Owl B renamed")
    field.send_keys("")  # Enter, which blurs and commits

    _, waited = wait_until(
        lambda: b.state()["device"]["name"] == "Owl B renamed", UI, "B did not take the new name"
    )
    measure(f"saved in {waited * 1000:.0f} ms")
    assert '"device_name": "Owl B renamed"' in (b.data / "settings.json").read_text()


@pytest.mark.xfail(
    strict=True,
    reason=(
        "the name reaches a peer in the Hello at the start of a connection and "
        "nothing re-sends it, so a connected peer keeps the old one. See the "
        "task E report, engine finding 2."
    ),
)
def test_11_the_new_name_reaches_a_connected_peer(pair):
    """A should show B's new name while the two are connected."""
    a, _ = pair
    wait_until(
        lambda: any(peer["name"] == "Owl B renamed" for peer in a.peers()),
        10.0,
        "A kept the old name",
    )


def test_11_the_new_name_reaches_the_peer_at_the_next_connect(pair, measure):
    """It does arrive, at the next connection, and the old name comes back."""
    a, b = pair
    b.restart()
    _, waited = wait_until(
        lambda: a.connected_to(b)
        and any(peer["name"] == "Owl B renamed" for peer in a.peers()),
        30.0,
        "A never took the new name",
    )
    measure(f"A showed the new name {waited:.1f} s after B came back")
    a.go("Devices")
    wait_until(
        lambda: "Owl B renamed" in a.texts(".devices-list .device-name"),
        UI,
        "the new name did not reach A's screen",
    )

    b.go("Settings")
    field = b.wait_for("input[aria-label='Device name']")
    b.type_into(field, "Owl B")
    field.send_keys("")
    wait_until(lambda: b.state()["device"]["name"] == "Owl B", UI, "the old name did not come back")
    b.name = "Owl B"
    b.restart()
    wait_until(
        lambda: a.connected_to(b) and any(peer["name"] == "Owl B" for peer in a.peers()),
        30.0,
        "A never took the old name back",
    )


def test_11_the_theme_toggle(pair):
    """Dark, light and back to the system, on the root element."""
    _, b = pair
    b.go("Settings")

    b.click(".segment", "Dark")
    wait_until(
        lambda: b.driver.execute_script("return document.documentElement.dataset.theme") == "dark",
        UI,
        "dark did not reach the document",
    )
    assert b.find(".segment", "Dark").get_attribute("aria-pressed") == "true"

    b.click(".segment", "Light")
    wait_until(
        lambda: b.driver.execute_script("return document.documentElement.dataset.theme") == "light",
        UI,
        "light did not reach the document",
    )

    b.click(".segment", "System")
    wait_until(
        lambda: b.driver.execute_script("return document.documentElement.dataset.theme") is None,
        UI,
        "the system choice did not clear the attribute",
    )


def test_11_the_title_bar_controls(pair):
    """Maximize swaps to Restore, and back."""
    a, _ = pair
    assert a.find(".titlebar") is not None
    assert a.text_of(a.find(".titlebar-title")) in {"Files", "Devices", "Settings"}

    a.click("[aria-label='Maximize']")
    wait_until(
        lambda: a.find("[aria-label='Restore']") is not None,
        UI,
        "the control did not swap to Restore",
    )
    a.click("[aria-label='Restore']")
    wait_until(
        lambda: a.find("[aria-label='Maximize']") is not None,
        UI,
        "the control did not swap back to Maximize",
    )


def test_12_two_beacons_on_one_port(pair, world, measure):
    """Both instances on beacon port 52735, and what Nearby shows.

    CONTRIBUTING says two sockets bound to one UDP port on one machine share
    the packets between them rather than both getting every one, so this is
    recorded rather than asserted. What is asserted is that the shared port
    does not disturb the link the two already have.
    """
    a, b = pair
    b.set_beacon_port(a.beacon_port)
    b.restart()
    wait_until(lambda: b.connected_to(a), 30.0, "B did not reconnect after the beacon change")

    heard = {"a": [], "b": []}
    deadline = time.monotonic() + 15.0
    while time.monotonic() < deadline:
        heard["a"] = a.state()["nearby"]
        heard["b"] = b.state()["nearby"]
        if heard["a"] or heard["b"]:
            break
        time.sleep(0.5)

    measure(f"nearby on A {len(heard['a'])}, on B {len(heard['b'])}")
    world["nearby_on_one_port"] = {side: len(items) for side, items in heard.items()}

    assert a.connected_to(b) and b.connected_to(a)

    b.set_beacon_port(52745)
    b.restart()
    wait_until(lambda: b.connected_to(a), 30.0, "B did not reconnect after the beacon change back")
