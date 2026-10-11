"""Regression checks for asynchronous saves and SSH fixture cleanup."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest

from e2e_support import config_revision, reap_shell, wait_for_config


class AcceptanceSupportTests(unittest.TestCase):
    def test_waits_for_delayed_atomic_save_with_missing_language(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.toml"
            path.write_text('theme = "Tokyo Night"\n')
            previous = config_revision(path)

            def save():
                time.sleep(0.2)
                temporary = path.with_suffix(".tmp")
                temporary.write_text('theme = "Tokyo Night"\nlanguage = "en"\n')
                temporary.replace(path)

            worker = threading.Thread(target=save)
            worker.start()
            try:
                saved = wait_for_config(
                    path, lambda cfg: cfg.get("language") == "en",
                    previous=previous, pump=time.sleep, timeout=2,
                )
                self.assertEqual(saved["language"], "en")
            finally:
                worker.join()

    def test_unchanged_config_does_not_count_as_a_save(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.toml"
            path.write_text('language = "en"\n')
            previous = config_revision(path)
            with self.assertRaisesRegex(AssertionError, "save did not complete"):
                wait_for_config(
                    path, lambda cfg: cfg.get("language") == "en",
                    previous=previous, pump=time.sleep, timeout=0.1,
                )
            temporary = path.with_suffix(".tmp")
            temporary.write_text(path.read_text())
            temporary.replace(path)
            saved = wait_for_config(
                path, lambda cfg: cfg.get("language") == "en",
                previous=previous, pump=time.sleep, timeout=1,
            )
            self.assertEqual(saved["language"], "en")

    @unittest.skipUnless(os.name == "posix", "fixture cleanup requires POSIX signals")
    def test_reaps_shell_when_original_process_group_is_missing(self):
        process = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
        try:
            self.assertNotEqual(os.getpgid(process.pid), process.pid)
            reap_shell(process)
            self.assertIsNotNone(process.returncode)
        finally:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)

    @unittest.skipUnless(os.name == "posix", "fixture cleanup requires POSIX signals")
    def test_reaps_shell_in_its_own_process_group(self):
        process = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(60)"], start_new_session=True,
        )
        try:
            self.assertEqual(os.getpgid(process.pid), process.pid)
            reap_shell(process)
            self.assertIsNotNone(process.returncode)
        finally:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)
