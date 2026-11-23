"""Failure handling, complete evidence, and byte-exact README update checks."""
import copy
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import render
import run


class RunnerTests(unittest.TestCase):
    def test_real_child_output_and_nonzero_exit_are_checked(self):
        self.assertGreater(run.measure([sys.executable, "-c", "print(42)"], "42\n", 5), 0)
        self.assertGreater(run.measure([sys.executable, "-c",
                                       "import sys; sys.stdout.buffer.write(b'42\\r\\n')"], "42\n", 5), 0)
        with self.assertRaisesRegex(RuntimeError, "wrong output"):
            run.measure([sys.executable, "-c", "print(41)"], "42\n", 5)
        with self.assertRaisesRegex(RuntimeError, "exit 7"):
            run.measure([sys.executable, "-c", "raise SystemExit(7)"], "", 5)

    def test_timeout_terminates_and_reports_child(self):
        with self.assertRaisesRegex(RuntimeError, "timeout"):
            run.measure([sys.executable, "-c", "import time; time.sleep(30)"], "", 0.05)

    def test_missing_runtime_is_explicit(self):
        commands, missing = run.resolve_commands("/not/a/scripta", "/not/a/php", sys.executable)
        self.assertEqual(set(commands), {"python"})
        self.assertEqual(set(missing), {"scripta", "php"})

    def test_failure_writes_failed_evidence_and_missing_runtime_writes_partial(self):
        with tempfile.TemporaryDirectory() as directory:
            parser = lambda output, python: type("Args", (), dict(
                scripta="/not/a/scripta", php="/not/a/php", python=python,
                output=Path(directory) / output, print_report=False,
                samples=1, warmups=0, seed=1, timeout=5))()
            args = parser("partial.json", "/not/a/python")
            self.assertEqual(run.run(args)["status"], "partial")
            args = parser("failed.json", sys.executable)
            with patch.object(run, "measure", side_effect=RuntimeError("wrong output")):
                with self.assertRaisesRegex(RuntimeError, "wrong output"):
                    run.run(args)
            self.assertEqual(json.loads(args.output.read_text())["status"], "failed")

    def test_optional_output_and_print_report(self):
        for flags in ([], ["--print"], ["--print", "--output", "report.json"]):
            with self.subTest(flags=flags), tempfile.TemporaryDirectory() as directory:
                output = Path(directory) / "report.json"
                arguments = [str(output) if flag == "report.json" else flag for flag in flags]
                stdout, stderr = io.StringIO(), io.StringIO()
                with patch.object(sys, "argv", ["run.py", "--scripta", "/not/a/scripta",
                                               "--php", "/not/a/php", "--samples", "1",
                                               "--warmups", "0", *arguments]), \
                        patch.object(run, "WORKLOADS", (run.WORKLOADS[-1],)), \
                        patch.object(sys, "stdout", stdout), patch.object(sys, "stderr", stderr):
                    with self.assertRaises(SystemExit) as exit_status:
                        run.main()
                self.assertEqual(exit_status.exception.code, 1)  # Missing runtimes remain an error.
                if "--print" in flags:
                    report = json.loads(stdout.getvalue())
                    self.assertEqual(report["status"], "partial")
                    self.assertEqual(len(report["results"]), 1)
                    self.assertIn("startup", stderr.getvalue())
                    if "--output" in flags:
                        self.assertEqual(json.loads(output.read_text()), report)
                else:
                    self.assertIn("startup", stdout.getvalue())
                self.assertEqual(output.exists(), "--output" in flags)

    def test_print_preserves_failed_report_without_output_file(self):
        args = type("Args", (), dict(scripta="/not/a/scripta", php="/not/a/php",
                                    python=sys.executable, output=None, print_report=True,
                                    samples=1, warmups=0, seed=1, timeout=5))()
        stdout = io.StringIO()
        with patch.object(sys, "stdout", stdout), \
                patch.object(run, "measure", side_effect=RuntimeError("wrong output")):
            with self.assertRaisesRegex(RuntimeError, "wrong output"):
                run.run(args)
        self.assertEqual(json.loads(stdout.getvalue())["status"], "failed")


class RenderTests(unittest.TestCase):
    def report(self):
        return {"schema": 1, "status": "complete", "unavailable": {}, "samples": 3,
                "warmups": 2, "date_utc": "2026-11-22", "cpu": "test CPU", "machine": "arm64",
                "platform": "test OS", "results": [
                    {"workload": name, "label": label, "size": size, "language": language,
                     "observations_ms": [3.0, 1.0, 2.0], "median_ms": 2.0}
                    for name, label, size, _ in run.WORKLOADS for language in run.LANGUAGES]}

    def test_table_uses_medians_without_links_to_local_evidence_or_runtime_versions(self):
        text = render.render(self.report())
        self.assertIn("| PHP (ms) | Python (ms) | Scripta (ms) |", text)
        self.assertEqual(text.count("| 2.000 | 2.000 | 2.000 |"), len(run.WORKLOADS))
        self.assertIn("](benchmarks/comparison/README.md)", text)
        self.assertIn("retained locally in ignored benchmark JSON files", text)
        self.assertNotIn(".json)", text)
        self.assertIn("Measured locally,", text)
        for detail in ("test CPU", "test OS", "2026-11-22", "ARM64", "arm64"):
            self.assertNotIn(detail, text)

    def test_performance_reference_contains_only_the_same_table(self):
        report = self.report()
        summary = render.render(report)
        table = render.render(report, table_only=True)
        self.assertEqual(table.splitlines(), [line for line in summary.splitlines() if line.startswith("|")])

    def test_cli_updates_both_documents_and_validates_before_writing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            evidence = root / "evidence.json"
            evidence.write_text(json.dumps(self.report()))
            readme = root / "README.md"
            performance = root / "docs" / "performance.md"
            performance.parent.mkdir()
            original = (render.START + "\nold\n" + render.END + "\n").encode()
            readme.write_bytes(original)
            performance.write_bytes(original.replace(b"\n", b"\r\n"))
            args = ["render.py", str(evidence), "--readme", str(readme), "--performance", str(performance)]
            with patch.object(sys, "argv", args):
                render.main()
            self.assertNotIn("](evidence.json)", readme.read_text())
            self.assertIn("retained locally", readme.read_text())
            self.assertEqual(json.loads(evidence.read_text()), self.report())
            expected = render.replace_section(original.replace(b"\n", b"\r\n"),
                                              render.render(self.report(), table_only=True))
            self.assertEqual(performance.read_bytes(), expected)
            # An invalid second destination must leave the first untouched.
            readme.write_bytes(original)
            performance.write_bytes(b"missing markers")
            with patch.object(sys, "argv", args), self.assertRaises(ValueError):
                render.main()
            self.assertEqual(readme.read_bytes(), original)

    def test_incomplete_duplicate_corrupt_and_failed_evidence_rejected(self):
        cases = []
        report = self.report(); report["results"].pop(); cases.append(report)
        report = self.report(); report["status"] = "failed"; cases.append(report)
        report = self.report(); report["results"].append(copy.deepcopy(report["results"][0])); cases.append(report)
        for value in (float("nan"), -1, 0):
            report = self.report(); report["results"][0]["observations_ms"][0] = value; cases.append(report)
        report = self.report(); report["results"][0]["median_ms"] = 99; cases.append(report)
        for report in cases:
            with self.subTest(report=report), self.assertRaises(ValueError):
                render.render(report)

    def test_readme_updates_preserve_lf_and_crlf_bytes(self):
        for newline in (b"\n", b"\r\n"):
            before = newline.join([b"prefix", render.START.encode(), b"old", render.END.encode(), b"suffix", b""])
            expected = newline.join([b"prefix", render.START.encode(), b"new", b"table", render.END.encode(), b"suffix", b""])
            self.assertEqual(render.replace_section(before, "new\ntable\n"), expected)
        with self.assertRaises(ValueError):
            render.replace_section(b"no markers", "table")


if __name__ == "__main__":
    unittest.main()
