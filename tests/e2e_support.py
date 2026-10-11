"""Standard-library helpers for deterministic interactive acceptance."""

import os
import signal
import time
import tomllib


def config_revision(path):
    stat = path.stat()
    return stat.st_dev, stat.st_ino, stat.st_mtime_ns, stat.st_ctime_ns


def wait_for_config(path, predicate, *, previous, pump, timeout=10):
    """Wait for an atomic save, including saves whose contents do not change."""
    end = time.monotonic() + timeout
    last = None
    while time.monotonic() < end:
        pump(0.05)
        if config_revision(path) != previous:
            last = tomllib.loads(path.read_text())
            if predicate(last):
                return last
    raise AssertionError(f"Configuration save did not complete: {path}; last value: {last!r}")


def reap_shell(process):
    """Kill the fixture process group and its shell, then collect its exit status."""
    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        # Interactive Bash can change process groups. A missing original group
        # does not imply that the shell itself exited.
        process.kill()
    process.wait(timeout=5)
