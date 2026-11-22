"""Reject incomplete or corrupt execution-only evidence without timing thresholds."""
import unittest

import execution as execution_profile


class ProfileTests(unittest.TestCase):
    def observations(self):
        return {stage: [3.0, 1.0, 2.0] for stage in execution_profile.STAGES}

    def test_raw_observations_and_medians_are_preserved(self):
        result = execution_profile.validate_observations(self.observations(), 3)
        for row in result.values():
            self.assertEqual(row["observations_ms"], [3.0, 1.0, 2.0])
            self.assertEqual((row["min_ms"], row["median_ms"], row["max_ms"]), (1.0, 2.0, 3.0))

    def test_missing_stages_wrong_counts_and_invalid_values_are_rejected(self):
        cases = []
        missing = self.observations(); missing.pop("execute_ms"); cases.append(missing)
        for value in (float("nan"), float("inf"), -1, True, "1"):
            result = self.observations(); result["execute_ms"][0] = value; cases.append(result)
        result = self.observations(); result["execute_ms"].pop(); cases.append(result)
        result = self.observations(); result["unexpected"] = [1, 2, 3]; cases.append(result)
        for result in cases:
            with self.subTest(result=result), self.assertRaises(ValueError):
                execution_profile.validate_observations(result, 3)


class NativeAllocationReportTests(unittest.TestCase):
    def test_recorded_totals_are_parsed_without_treating_them_as_live_memory(self):
        from native_profile import allocation_totals
        report = """High water mark of allocated heap + VM memory: 1 MB
Current total allocated heap + VM memory: 317 KB
Cumulative total allocated heap + VM memory: 860.9 MB
Total heap + VM memory operations: 10,972,788  allocations: 5,487,258  deallocations: 5,485,530
"""
        result = allocation_totals(report)
        self.assertEqual(result["recorded_allocations"], 5487258)
        self.assertEqual(result["recorded_cumulative_size"], "860.9 MB")
        self.assertEqual(result["recorded_operations"], result["recorded_allocations"] + result["recorded_deallocations"])

    def test_missing_native_totals_are_not_silently_accepted(self):
        from native_profile import allocation_totals
        with self.assertRaises(ValueError):
            allocation_totals("Can't show high water mark in lite mode")


if __name__ == "__main__":
    unittest.main()
