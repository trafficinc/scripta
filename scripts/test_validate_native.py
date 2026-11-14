"""Check evidence parsing and static-CRT rejection without native toolchains."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from validate_native import ROOT, Validation, test_counts, windows_imports


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

    def test_windows_style_checkout_keeps_scripts_lf_and_snapshots_byte_exact(self):
        fixtures = {
            'examples/report.scripta': b'<?scripta\necho "ok";\n',
            'playground/report.scta': b'<?scripta\necho "ok";\n',
            'examples/legacy.scr': b'<?scr\necho "ok";\n',
            'crates/scr-cli/tests/snapshots/formatter/newlines.input.scripta': b'<?scr\r\necho "first\r\nsecond";\n',
            'crates/scr-cli/tests/snapshots/formatter/newlines.expected.scripta': b'<?scr\necho "first\r\nsecond";\n',
            'benchmarks/fixtures/newlines.scripta': b'<?scripta\r\necho "ok";\n',
        }
        environment = {**os.environ, 'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': os.devnull}
        with tempfile.TemporaryDirectory() as directory:
            checkout = Path(directory) / 'source'
            checkout.mkdir()
            (checkout / '.gitattributes').write_bytes((ROOT / '.gitattributes').read_bytes())
            for name, data in fixtures.items():
                path = checkout / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            def git(*arguments):
                subprocess.run(['git', '-C', str(checkout), '-c', 'core.autocrlf=true', *arguments],
                               env=environment, check=True, capture_output=True)
            git('init', '--quiet')
            git('add', '.')
            output = Path(directory) / 'export'
            git('checkout-index', '--all', '--prefix=' + output.as_posix() + '/')
            for name, expected in fixtures.items():
                self.assertEqual((output / name).read_bytes(), expected, name)

    def test_host_mismatch_stops_before_tests_and_packaging(self):
        with tempfile.TemporaryDirectory() as directory:
            validation = Validation('x86_64-pc-windows-msvc', Path(directory))
            with patch.object(validation, 'run', side_effect=['', 'host: aarch64-apple-darwin\n']) as run:
                with self.assertRaisesRegex(RuntimeError, 'host differs'):
                    validation.execute()
                self.assertEqual(run.call_count, 2)

    def test_mac_release_builds_keep_the_declared_deployment_target(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.dict(os.environ, {'MACOSX_DEPLOYMENT_TARGET': '99.0'}):
                validation = Validation('aarch64-apple-darwin', Path(directory))
            self.assertEqual(validation.environment['MACOSX_DEPLOYMENT_TARGET'], '15.0')

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
