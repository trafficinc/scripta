"""Check evidence parsing and static-CRT rejection without native toolchains."""
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from validate_native import Validation, test_counts, windows_imports


class EvidenceTests(unittest.TestCase):
    def test_counts_accept_complete_success_and_reject_failed_or_ignored_results(self):
        self.assertEqual(test_counts('test result: ok. 12 passed; 0 failed; 0 ignored;\ntest result: ok. 2 passed; 0 failed; 0 ignored;'), {'passed': 14, 'failed': 0, 'ignored': 0})
        for text in ['', 'test result: FAILED. 1 passed; 1 failed; 0 ignored;', 'test result: ok. 1 passed; 0 failed; 1 ignored;']:
            with self.assertRaises(RuntimeError):
                test_counts(text)

    def test_static_crt_audit_rejects_dynamic_runtime_dlls(self):
        self.assertEqual(windows_imports('    KERNEL32.dll\n    bcrypt.dll\n'), ['bcrypt.dll', 'KERNEL32.dll'])
        for name in ['VCRUNTIME140.dll', 'MSVCP140.dll', 'ucrtbase.dll', 'api-ms-win-crt-runtime-l1-1-0.dll', 'libcrypto-3.dll', 'sqlite3.dll']:
            with self.assertRaises(RuntimeError):
                windows_imports('    KERNEL32.dll\n    ' + name + '\n')
        with self.assertRaises(RuntimeError):
            windows_imports('no dependency evidence')

    def test_host_mismatch_stops_before_tests_and_packaging(self):
        with tempfile.TemporaryDirectory() as directory:
            validation = Validation('x86_64-pc-windows-msvc', Path(directory))
            with patch.object(validation, 'run', side_effect=['', 'host: aarch64-apple-darwin\n']) as run:
                with self.assertRaisesRegex(RuntimeError, 'host differs'):
                    validation.execute()
                self.assertEqual(run.call_count, 2)

    def test_failed_command_is_saved_with_log_and_nonzero_status(self):
        with tempfile.TemporaryDirectory() as directory:
            validation = Validation('aarch64-apple-darwin', Path(directory))
            with self.assertRaises(RuntimeError):
                validation.run('failure', ['__missing_validation_executable__'])
            self.assertTrue((Path(directory) / 'logs/failure.log').exists())
            self.assertEqual(validation.record['steps'][-1]['exit_code'], None)
            self.assertIn('error', validation.record['steps'][-1])


if __name__ == '__main__':
    unittest.main()
