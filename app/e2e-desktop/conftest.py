"""Fixtures for the desktop end to end suite, and the table it prints.

The two instances are built once for the whole session and carried through
every step, because the steps are a story: what step 1 paired, step 10
reconnects. Running one test on its own therefore runs the ones before it that
it depends on, which is why the file names them in order.
"""

from __future__ import annotations

import os
import shutil
import signal
import time
from pathlib import Path

import pytest

from owl import BINARY, Instance, drivers_on, processes_for

# Ports. Each instance needs four of its own: the engine's TCP port, its UDP
# beacon, the tauri-driver in front of it and the WebKitWebDriver behind that.
# The two engines' ports. A installed copy of the app, or anything else already
# listening on 52734, stops the suite before the first test, so the pair can be
# moved with OWL_E2E_PORT_BASE. The default is the app's own port, which is what
# the numbers in the README say.
PORT_BASE = int(os.environ.get("OWL_E2E_PORT_BASE", "52734"))

PORTS = {
    "a": {
        "tcp_port": PORT_BASE,
        "beacon_port": PORT_BASE + 1,
        "driver_port": 4444,
        "native_port": 4464,
    },
    "b": {
        # Ten apart, so that the two beacons cannot hear each other and B has
        # to pair by typing A's address, which is the step the suite starts on.
        "tcp_port": PORT_BASE + 10,
        "beacon_port": PORT_BASE + 11,
        "driver_port": 4446,
        "native_port": 4466,
    },
}

WORK = Path(os.environ.get("OWL_E2E_WORK", "/tmp/owl-e2e-desktop"))

# What each step measured, by test id, for the table at the end.
NOTES: dict[str, list[str]] = {}
RESULTS: list[tuple[str, str, float]] = []


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption(
        "--keep-work",
        action="store_true",
        help="leave the two instances' data directories and folders behind",
    )


@pytest.fixture(scope="session")
def work(request: pytest.FixtureRequest) -> Path:
    if WORK.exists():
        shutil.rmtree(WORK)
    WORK.mkdir(parents=True)
    yield WORK
    if not request.config.getoption("--keep-work"):
        shutil.rmtree(WORK, ignore_errors=True)


@pytest.fixture(scope="session")
def pair(work: Path):
    """Instances A and B, started, with nothing paired yet."""
    assert BINARY.exists(), (
        f"no binary at {BINARY}. Build one with "
        "`cd app && npx tauri build --debug --no-bundle`, or point OWL_BINARY at it."
    )

    instances = [
        Instance(tag="a", name="Owl A", root=work, **PORTS["a"]),
        Instance(tag="b", name="Owl B", root=work, **PORTS["b"]),
    ]
    # Anything left over from an interrupted run still holds the ports.
    stale = drivers_on([port for group in PORTS.values() for port in group.values()])
    for instance in instances:
        stale += processes_for(instance.data)
    for pid in stale:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    if stale:
        time.sleep(1.0)

    for instance in instances:
        instance.start()
    try:
        yield instances[0], instances[1]
    finally:
        for instance in instances:
            instance.stop()


@pytest.fixture(scope="session")
def world() -> dict:
    """What one step learned and a later one checks. Kept small on purpose."""
    return {"status_seen": set()}


@pytest.fixture
def measure(request: pytest.FixtureRequest):
    """Records a number this step measured, for the table at the end."""

    def note(text: str) -> None:
        NOTES.setdefault(request.node.nodeid, []).append(text)

    return note


def pytest_runtest_logreport(report: pytest.TestReport) -> None:
    if report.when != "call":
        return
    outcome = report.outcome
    if hasattr(report, "wasxfail"):
        outcome = "xpass" if outcome == "passed" else "xfail"
    RESULTS.append((report.nodeid, outcome, report.duration))


def pytest_terminal_summary(terminalreporter) -> None:
    if not RESULTS:
        return
    rows = ["", "| Step | Result | Time | Measured |", "| --- | --- | --- | --- |"]
    for nodeid, outcome, duration in RESULTS:
        name = nodeid.split("::")[-1]
        notes = "; ".join(NOTES.get(nodeid, [])) or ""
        rows.append(f"| {name} | {outcome} | {duration:.1f} s | {notes} |")
    table = "\n".join(rows)
    terminalreporter.write_line(table)

    destination = os.environ.get("OWL_E2E_REPORT")
    if destination:
        Path(destination).write_text(f"{table}\n\nRun finished {time.strftime('%F %T')}\n")
        terminalreporter.write_line(f"table written to {destination}")
