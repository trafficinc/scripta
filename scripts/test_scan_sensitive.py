"""Exercise detection and report privacy using a real Gitleaks executable."""
import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from scan_sensitive import locations, main

GITLEAKS = shutil.which(os.environ.get('GITLEAKS_BIN', 'gitleaks'))


class SensitiveScanTests(unittest.TestCase):
    def test_report_omits_source_values_and_commit_identity(self):
        report = locations([{'File': '/repo/app.scripta', 'RuleID': 'test',
                             'StartLine': 3, 'Commit': 'abc', 'Secret': 'sensitive',
                             'Match': 'sensitive line', 'Author': 'private person',
                             'Email': 'private address', 'Message': 'private message'}],
                           'history', Path('/repo'))
        self.assertEqual(report, [{'scope': 'history', 'rule': 'test',
                                   'file': 'app.scripta', 'line': 3, 'commit': 'abc'}])

    @unittest.skipUnless(GITLEAKS, 'Set GITLEAKS_BIN to run real scanner tests')
    def test_lf_crlf_ignored_env_history_detection_and_redaction(self):
        for newline in (b'\n', b'\r\n'):
            with self.subTest(newline=newline), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                subprocess.run(['git', 'init', '-q', str(root)], check=True)
                subprocess.run(['git', '-C', str(root), 'config', 'core.autocrlf', 'false'],
                               check=True)
                (root / '.gitignore').write_text('.env\n.security-reports/\n')
                # Construct synthetic values so this test source is not a finding.
                email = 'scanner-fixture' + '@' + 'example.org'
                phone = '+1 ' + '202-555-0198'
                ssn = '-'.join(('123', '45', '6789'))
                token = 'ghp_' + 'Ab3dE5gH7jK9mN2pQ4sT6vW8yZ1cF0rL3xU5'
                payload = f'$email="{email}";\n$phone="{phone}";\n$ssn="{ssn}";\n$token="{token}";\n'
                (root / 'app.scripta').write_bytes(payload.encode().replace(b'\n', newline))
                subprocess.run(['git', '-C', str(root), 'add', '.'], check=True)
                subprocess.run(['git', '-C', str(root), '-c', 'user.name=Scanner fixture',
                                '-c', 'user.email=scanner@example.invalid', 'commit', '-qm',
                                'Synthetic scanner fixture'], check=True)
                (root / 'app.scripta').write_text('echo "clean";\n')
                (root / '.env').write_bytes(payload.encode().replace(b'\n', newline))
                console = io.StringIO()
                with contextlib.redirect_stdout(console):
                    code = main(['--root', str(root), '--gitleaks', GITLEAKS])
                self.assertEqual(code, 1)
                encoded = (root / '.security-reports/findings.json').read_text()
                report = json.loads(encoded)
                for scope in ('files', 'history'):
                    rules = {f['rule'] for f in report['findings'] if f['scope'] == scope}
                    self.assertTrue({'pii-email', 'pii-phone-labeled', 'pii-us-ssn',
                                     'github-pat'}.issubset(rules), rules)
                self.assertTrue(any(f['file'] == '.env' for f in report['findings']))
                for value in (email, phone, ssn, token):
                    self.assertNotIn(value, encoded + console.getvalue())

    @unittest.skipUnless(GITLEAKS, 'Set GITLEAKS_BIN to run real scanner tests')
    def test_clean_files_and_invalid_history_have_distinct_exit_codes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'clean.scripta').write_text('echo "hello";\n')
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(main(['--root', directory, '--scope', 'files',
                                       '--gitleaks', GITLEAKS]), 0)
            report = (root / '.security-reports/findings.json').read_bytes()
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(['--root', directory, '--scope', 'history',
                                       '--gitleaks', GITLEAKS]), 2)
            self.assertEqual((root / '.security-reports/findings.json').read_bytes(), report)


if __name__ == '__main__':
    unittest.main()
