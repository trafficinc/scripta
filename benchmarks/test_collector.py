"""Contract tests for benchmark result validation and summaries."""
import json
import subprocess
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from collector import run_case


class CollectorReportTests(unittest.TestCase):
    def setUp(self):
        self.args = SimpleNamespace(samples=5, warmups=1, timeout=10)
        self.row = {
            "case": "live", "size": 1000, "phase": "collect",
            "debug_assertions": False,
            "observations_ns": [500, 100, 400, 200, 300],
            "reclaimed_scopes": [0, 0, 0, 0, 0],
        }

    def run_row(self):
        result = SimpleNamespace(stdout=json.dumps(self.row))
        with patch("collector.subprocess.run", return_value=result):
            return run_case(Path("worker"), "live", 1000, self.args)

    def test_summary_retains_raw_samples_and_handles_unsorted_timings(self):
        row = self.run_row()
        self.assertEqual(row["observations_ns"], [500, 100, 400, 200, 300])
        self.assertEqual(row["median_ns"], 300)
        self.assertEqual(row["p95_ns"], 500)
        self.assertEqual(row["max_ns"], 500)
        self.assertEqual(row["median_ns_per_item"], 0.3)

    def test_debug_workers_cannot_silently_supply_performance_baselines(self):
        self.row["debug_assertions"] = True
        with self.assertRaises(RuntimeError):
            self.run_row()

    def test_wrong_workloads_and_incomplete_samples_are_rejected(self):
        for key, value in [("case", "mixed"), ("size", 1),
                           ("observations_ns", [1]), ("reclaimed_scopes", [])]:
            with self.subTest(key=key):
                original = self.row[key]
                self.row[key] = value
                with self.assertRaises(RuntimeError):
                    self.run_row()
                self.row[key] = original

    def test_worker_failure_is_not_reported_as_a_successful_measurement(self):
        with patch("collector.subprocess.run", side_effect=subprocess.CalledProcessError(1, "worker")):
            with self.assertRaises(subprocess.CalledProcessError):
                run_case(Path("worker"), "live", 1000, self.args)


if __name__ == "__main__":
    unittest.main()
