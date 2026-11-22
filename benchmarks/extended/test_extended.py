"""Extended runner correctness, failure handling, and resource cleanup."""
import copy
from contextlib import redirect_stdout
import importlib.util
import io
import json
import math
import os
from pathlib import Path
import socket
import shutil
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock, patch
import urllib.request

# Unique names avoid collisions with the existing comparison suite's run.py.
ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("extended_workloads", ROOT / "workloads.py")
workloads = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workloads)
with patch.dict(sys.modules, {"workloads": workloads}):
    spec = importlib.util.spec_from_file_location("extended_run", ROOT / "run.py")
    runner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runner)


class WorkerTests(unittest.TestCase):
    def test_python_numeric_workloads_at_boundaries(self):
        with tempfile.TemporaryDirectory() as directory:
            workloads.prepare(directory)
            for n in (1, 19, 20, 21, 97, 101):
                for case in workloads.CASES[1:5]:
                    command = [sys.executable, "-I", str(ROOT / "fixtures/worker.py"),
                               case, str(n), "2", directory, "", sys.executable]
                    observation, output = runner.measure(command, 5)
                    times = runner.validate(output, case, n, 2, observation["wall_ms"])
                    self.assertEqual(len(times), 2)
                    if hasattr(os, "wait4"):
                        self.assertGreater(observation["peak_rss_mib"], 0)

    def test_wrong_checksum_and_malformed_times_are_rejected(self):
        valid = {"case": "numeric_calls", "checksums": [workloads.expected("numeric_calls", 101)],
                 "elapsed_ms": [1.0]}
        for key, bad in (("checksums", [True]), ("checksums", [0]), ("elapsed_ms", []),
                         ("elapsed_ms", [True]), ("elapsed_ms", [-1]), ("elapsed_ms", [math.nan]),
                         ("elapsed_ms", [math.inf]), ("elapsed_ms", [200]), ("case", "files")):
            value = copy.deepcopy(valid)
            value[key] = bad
            with self.assertRaisesRegex(RuntimeError, "invalid worker output"):
                runner.validate(json.dumps(value).encode(), "numeric_calls", 101, 1, 10)
        for newline in (b"\n", b"\r\n"):
            self.assertEqual(runner.validate(json.dumps(valid).encode() + newline,
                                            "numeric_calls", 101, 1, 10), [1.0])
        with self.assertRaises(RuntimeError):
            runner.validate(b"noise", "startup", 1, 1, 10)

    @unittest.skipUnless(os.name == "posix", "POSIX process groups")
    def test_exited_group_race_is_reaped_but_live_permission_errors_fail(self):
        child = Mock(pid=123, poll=Mock(return_value=0))
        with patch.object(runner.os, "killpg", side_effect=[PermissionError(), PermissionError(), ProcessLookupError()]):
            runner.stop(child)
        child.wait.assert_called_once()
        child.kill.assert_not_called()
        child = Mock(pid=123, poll=Mock(return_value=None))
        with patch.object(runner.os, "killpg", side_effect=PermissionError()):
            with self.assertRaises(PermissionError):
                runner.stop(child)
        child.kill.assert_called_once()
        child.wait.assert_called_once()

    def test_missing_rss_support_still_measures_and_checks_output(self):
        with patch.object(runner, "WAIT4", None):
            observation, output = runner.measure([sys.executable, "-I", "-c", "print(42)"], 5)
        self.assertEqual(output, b"42\n")
        self.assertIsNone(observation["peak_rss_mib"])
        self.assertGreater(observation["wall_ms"], 0)

    def test_nonzero_exit_stderr_and_excess_output_are_failures(self):
        for code, message in [("raise SystemExit(7)", "exit 7"),
                              ("import sys;sys.stderr.write('warning')", "unexpected stderr"),
                              ("print('x'*1100000)", "output limit")]:
            with self.assertRaisesRegex(RuntimeError, message):
                runner.measure([sys.executable, "-I", "-c", code], 5)

    @unittest.skipUnless(os.name == "posix", "POSIX process-group cleanup")
    def test_timeout_and_early_parent_exit_kill_descendants(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "survived"
            ready = Path(directory) / "ready"
            grandchild = f"from pathlib import Path;import time;Path({str(ready)!r}).touch();time.sleep(0.5);Path({str(marker)!r}).touch()"
            for early_exit in (False, True):
                ready.unlink(missing_ok=True)
                parent = ("import subprocess,sys,time;from pathlib import Path;"
                          f"subprocess.Popen([sys.executable,'-I','-c',{grandchild!r}]);"
                          f"ready=Path({str(ready)!r});\n"
                          "while not ready.exists(): time.sleep(0.001)\n" +
                          ("raise SystemExit(7)" if early_exit else "time.sleep(30)"))
                with self.assertRaisesRegex(RuntimeError, "exit 7" if early_exit else "timeout"):
                    runner.measure([sys.executable, "-I", "-c", parent], 5 if early_exit else 0.25)
                self.assertTrue(ready.exists(), "grandchild must start before testing cleanup")
                time.sleep(0.6)
                self.assertFalse(marker.exists())


class FixturesTests(unittest.TestCase):
    def test_explicit_lf_and_crlf_inputs_have_the_same_result(self):
        commands = {"python": [sys.executable, "-I"]}
        if shutil.which("php"):
            commands["php"] = [shutil.which("php"), "-n"]
        binary = os.environ.get("SCRIPTA_BENCH_BINARY")
        if binary:
            commands["scripta"] = [str(Path(binary).resolve()), "run"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            expected = workloads.prepare(root)
            originals = {name: (root / name).read_bytes() for name in ("input.txt", "input.csv")}
            for newline in (b"\n", b"\r\n"):
                for name, contents in originals.items():
                    # Use explicit byte fixtures for both newline conventions.
                    normalized = contents.replace(b"\r\n", b"\n")
                    if name == "input.csv":
                        # Record separators change; the quoted field retains its LF content.
                        records = contents.replace(b"\r\n", newline)
                    else:
                        records = normalized.replace(b"\n", newline)
                    (root / name).write_bytes(records)
                for language, command in commands.items():
                    for case in ("files", "csv_json"):
                        fixture = ROOT / "fixtures" / ("worker." + runner.EXTENSIONS[language])
                        observation, output = runner.measure(command + [str(fixture), case, "1", "2",
                                                                        directory, "", sys.executable], 5)
                        runner.validate(output, case, 1, 2, observation["wall_ms"])
                    self.assertEqual((root / ("output-" + language + ".txt")).read_bytes(), expected)


    def test_chunked_binary_http_and_cleanup_on_failure(self):
        address = None
        with self.assertRaisesRegex(ValueError, "fixture failure"):
            with workloads.local_http(True) as (url, server):
                address = server.server_address
                with urllib.request.urlopen(url, timeout=5) as response:
                    self.assertEqual(response.headers["Transfer-Encoding"], "chunked")
                    self.assertEqual(response.read(), workloads.PAYLOAD)
                self.assertEqual(server.requests, 1)
                raise ValueError("fixture failure")
        with socket.socket() as client:
            self.assertNotEqual(client.connect_ex(address), 0)


class ReportTests(unittest.TestCase):
    def args(self, *extra):
        return runner.parser().parse_args(["--scripta", "/not/a/scripta", "--php", "/not/a/php",
                                          "--python", sys.executable, "--samples", "1", "--warmups", "0",
                                          "--iterations", "101", "--batches", "2", *extra])

    def test_partial_run_checks_all_python_workloads_and_has_separate_startup(self):
        with context_stdout():
            report = runner.run(self.args())
        self.assertEqual(report["status"], "partial")
        self.assertEqual(set(report["unavailable"]), {"scripta", "php"})
        self.assertEqual(len(report["results"]), len(workloads.CASES))
        startup = report["results"][0]
        self.assertEqual(startup["workload"], "startup")
        self.assertNotIn("execution_ms_per_batch", startup)
        for row in report["results"][1:]:
            self.assertEqual(len(row["observations"][0]["execution_batches_ms"]), 2)
            self.assertGreater(row["units_per_second"], 0)

    def test_no_runtimes_is_partial_without_opening_http_sockets(self):
        args = self.args("--python", "/not/a/python")
        with context_stdout():
            report = runner.run(args)
        self.assertEqual(report["status"], "partial")
        self.assertEqual(report["results"], [])
        self.assertEqual(set(report["unavailable"]), {"scripta", "php", "python", "subprocess"})

    def test_failure_preserves_observations_and_cleans_temporary_inputs(self):
        directories = []
        def prepared(directory):
            directories.append(directory)
            return workloads.prepare(directory)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "failed.json"
            args = self.args("--workload", "numeric_calls", "--output", str(output), "--samples", "2")
            good = json.dumps({"case": "numeric_calls", "elapsed_ms": [1, 1],
                               "checksums": [workloads.expected("numeric_calls", 101)] * 2}).encode()
            with patch.object(runner, "prepare", side_effect=prepared), patch.object(runner, "measure", side_effect=[
                    ({"wall_ms": 5, "peak_rss_mib": None}, good),
                    ({"wall_ms": 5, "peak_rss_mib": None}, b"{}")]):
                with self.assertRaises(RuntimeError):
                    runner.run(args)
            report = json.loads(output.read_text())
            self.assertEqual(report["status"], "failed")
            self.assertEqual(len(report["results"][0]["observations"]), 1)
            self.assertEqual(report["active"]["round"], 1)
            self.assertTrue(directories)
            self.assertTrue(all(not p.exists() for p in directories))
            original = output.read_bytes()
            with self.assertRaisesRegex(ValueError, "already exists"):
                runner.run(args)
            self.assertEqual(output.read_bytes(), original)

    def test_sqlite_unavailable_is_explicit_and_other_workloads_continue(self):
        original = runner.measure
        def missing_sqlite(command, timeout):
            if command[-2:] == ["-c", "import sqlite3"]:
                raise RuntimeError("sqlite module unavailable")
            return original(command, timeout)
        with patch.object(runner, "measure", side_effect=missing_sqlite), context_stdout():
            report = runner.run(self.args("--workload", "sqlite", "--workload", "startup"))
        self.assertEqual(report["status"], "partial")
        self.assertIn("python", report["unavailable"]["sqlite"])
        self.assertEqual([r["workload"] for r in report["results"]], ["startup"])

    def test_invalid_cli_counts_and_nonfinite_deadlines_are_rejected(self):
        for option, value in (("--samples", "0"), ("--warmups", "-1"), ("--batches", "0"),
                              ("--iterations", "1000001"), ("--timeout", "nan"), ("--timeout", "inf")):
            with patch.object(sys, "argv", ["run.py", "--scripta", "unused", option, value]), patch("sys.stderr", io.StringIO()):
                with self.assertRaises(SystemExit) as failure:
                    runner.main()
                self.assertEqual(failure.exception.code, 2)


# Keep expected console output out of unittest output; assertions inspect the report.
def context_stdout():
    return redirect_stdout(io.StringIO())


if __name__ == "__main__":
    unittest.main()
