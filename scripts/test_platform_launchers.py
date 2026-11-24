"""Launcher quoting, evidence preservation, and failure propagation without Rust."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(sys.platform in ("darwin", "linux"), "Bash launchers run on Linux/macOS")
class BashLauncherTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="scripta-launcher-")
        self.addCleanup(self.scratch.cleanup)
        self.directory = Path(self.scratch.name)
        self.binary = self.directory / 'binary with spaces é "quote"'
        self.binary.write_text(
            '#!/bin/sh\n'
            'case "$1" in\n'
            'version) echo "fixture version"; exit "${FIXTURE_VERSION_EXIT:-0}" ;;\n'
            'test) printf "%s\\n" "$SCRIPTA_PLATFORM_BINARY" "$SCRIPTA_PLATFORM_OS" '
            '"$SCRIPTA_PLATFORM_ARCH" "$SCRIPTA_PLATFORM_FIXTURES" > "$FIXTURE_CAPTURE"; '
            'echo "{}"; echo "fixture stderr" >&2; exit "$FIXTURE_TEST_EXIT" ;;\n'
            'run) exit "$FIXTURE_SUMMARY_EXIT" ;;\n'
            'esac\nexit 99\n', encoding="utf-8", newline="\n")
        self.binary.chmod(0o755)
        self.results = self.directory / "results with spaces é"
        self.capture = self.directory / "capture.txt"
        self.mode = "macos" if sys.platform == "darwin" else "linux"

    def launch(self, test_exit=0, summary_exit=0, extra=(), mode=None):
        env = dict(os.environ, FIXTURE_CAPTURE=str(self.capture),
                   FIXTURE_TEST_EXIT=str(test_exit), FIXTURE_SUMMARY_EXIT=str(summary_exit))
        return subprocess.run(
            ["bash", str(ROOT / "scripts" / f"test-platform-{mode or self.mode}.sh"),
             "--scripta", str(self.binary), "--output", str(self.results), *extra],
            cwd=self.directory, env=env, capture_output=True, text=True, timeout=20)

    def test_unicode_quoted_paths_and_different_working_directory(self):
        child = self.launch()
        self.assertEqual(child.returncode, 0, child.stdout + child.stderr)
        values = self.capture.read_text(encoding="utf-8").splitlines()
        self.assertEqual(values[0], str(self.binary.resolve()))
        self.assertEqual(values[1], self.mode)
        self.assertIn(values[2], ("x86_64", "aarch64"))
        self.assertEqual(values[3], str(ROOT / "tests/platform/fixtures"))
        self.assertEqual((self.results / "stderr.txt").read_text(), "fixture stderr\n")
        self.assertTrue((self.results / "report.json").is_file())
        self.assertTrue((self.results / "version.txt").is_file())

    def test_runner_failure_is_not_hidden_by_successful_summary(self):
        child = self.launch(test_exit=7)
        self.assertEqual(child.returncode, 7, child.stdout + child.stderr)
        self.assertTrue((self.results / "report.json").is_file())

    def test_bad_summary_is_not_reported_as_success(self):
        child = self.launch(summary_exit=9)
        self.assertEqual(child.returncode, 9, child.stdout + child.stderr)

    def test_existing_evidence_is_unchanged(self):
        self.results.mkdir()
        report = self.results / "report.json"
        original = b"historical evidence\r\n"
        report.write_bytes(original)
        child = self.launch()
        self.assertNotEqual(child.returncode, 0)
        self.assertEqual(report.read_bytes(), original)
        self.assertFalse(self.capture.exists())

    def test_wrong_os_and_conflicting_arguments_fail_before_running(self):
        child = self.launch(mode="linux" if self.mode == "macos" else "macos")
        self.assertEqual(child.returncode, 2)
        self.assertFalse(self.results.exists())
        child = self.launch(extra=("--build",))
        self.assertEqual(child.returncode, 2)
        self.assertFalse(self.results.exists())

    def test_newline_attributes_are_explicit(self):
        attributes = (ROOT / ".gitattributes").read_text()
        self.assertIn("*.sh text eol=lf", attributes)
        self.assertIn("*.ps1 text eol=lf", attributes)
        self.assertIn("*.cmd text eol=crlf", attributes)


if __name__ == "__main__":
    unittest.main()
