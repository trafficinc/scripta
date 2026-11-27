"""Public downloads contain runtime payloads only, even with forged native metadata."""
import json
from pathlib import Path
import stat
import tempfile
import unittest
import warnings
import zipfile
from unittest.mock import Mock

import release_assets as policy
from publish_release import digest, prepare, publish
import test_publish_release as fixtures

REVISION = fixtures.REVISION


class RuntimeAssetsTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        fixtures.ReleaseTests().fixture(self.root / 'inputs')
        self.target = 'x86_64-pc-windows-msvc'
        self.name = policy.package_names('0.1.0')[self.target]
        self.native = self.root / 'inputs' / ('native-validation-' + self.target + '-1')
        self.archive = self.native / 'standard' / self.name
        self.original = self.archive.read_bytes()
        self.prefix = f'scripta-0.1.0-{self.target}/'

    def test_each_platform_has_executables_and_legal_notices_only(self):
        for target, name in policy.package_names('0.1.0').items():
            path = self.root / 'inputs' / ('native-validation-' + target + '-1') / 'standard' / name
            details = policy.inspect_package(path, '0.1.0', target)
            primary = 'scripta.exe' if target == self.target else 'scripta'
            self.assertIn(primary, details['files'])
            self.assertIn('LICENSE', details['files'])
            self.assertIn('THIRD_PARTY_NOTICES.txt', details['files'])
            self.assertFalse(any(name.endswith('.json') for name in details['files']))

    def test_docs_examples_playgrounds_and_internal_files_cannot_enter_archives(self):
        for name in ['README.md', 'docs/install.md', 'examples/hello.scripta',
                     'playground/demo.scripta', 'release.json', 'validation.json',
                     'validation-evidence.zip', 'logs/build.log', 'smoke.json',
                     'third-party/test-1/provenance.json', 'third-party/test-1/LICENSE.log']:
            with self.subTest(name=name):
                self.archive.write_bytes(self.original)
                with zipfile.ZipFile(self.archive, 'a') as archive:
                    archive.writestr(self.prefix + name, b'not a runtime file')
                with self.assertRaises(ValueError):
                    policy.inspect_package(self.archive, '0.1.0', self.target)
                # Updating the trusted-looking native hash must not bypass the content policy.
                path = self.native / 'validation.json'
                record = json.loads(path.read_bytes())
                record['packages']['standard']['archive_sha256'] = digest(self.archive)
                path.write_bytes(json.dumps(record).encode())
                smoke_path = self.archive.with_name(self.name + '.validation.json')
                smoke = json.loads(smoke_path.read_bytes())
                smoke['archive_sha256'] = digest(self.archive)
                smoke_path.write_bytes(json.dumps(smoke).encode())
                with self.assertRaises(ValueError):
                    prepare(self.root / 'inputs', self.root / 'assets', REVISION, 'v0.1.0', '0.1.0')
                self.assertFalse((self.root / 'assets').exists())

    def test_duplicate_unsafe_and_nonregular_members_are_rejected(self):
        for name, mode in [('scripta.exe', stat.S_IFREG), ('../outside', stat.S_IFREG),
                           ('third-party/test-1/LICENSE', stat.S_IFLNK)]:
            with self.subTest(name=name, mode=mode):
                self.archive.write_bytes(self.original)
                entry = zipfile.ZipInfo(self.prefix + name)
                entry.external_attr = (mode | 0o644) << 16
                with warnings.catch_warnings(), zipfile.ZipFile(self.archive, 'a') as archive:
                    warnings.simplefilter('ignore', UserWarning)
                    archive.writestr(entry, b'bad')
                with self.assertRaises(ValueError):
                    policy.inspect_package(self.archive, '0.1.0', self.target)

    def test_publication_refuses_internal_assets_before_contacting_github(self):
        output = self.root / 'assets'
        manifest, notes = prepare(self.root / 'inputs', output, REVISION, 'v0.1.0', '0.1.0')
        for name in ['validation-evidence.zip', 'validation.json', 'build.log',
                     'release-manifest.json', 'smoke.json', self.name + '.package.json']:
            path = output / name
            path.write_bytes(b'internal')
            api = Mock()
            with self.subTest(name=name), self.assertRaises(ValueError):
                publish('owner/repo', manifest, output, notes, api)
            api.assert_not_called()
            path.unlink()
        self.assertEqual(len(list(output.iterdir())), 4)
        self.assertTrue((policy.validation_directory(output) / 'release-manifest.json').is_file())

    def test_unexpected_remote_draft_assets_prevent_upload_or_publication(self):
        output = self.root / 'assets'
        manifest, notes = prepare(self.root / 'inputs', output, REVISION, 'v0.1.0', '0.1.0')
        import subprocess
        for stale_before_upload in (True, False):
            calls = []
            def run(arguments, **kwargs):
                calls.append(arguments)
                if arguments[2] == 'view':
                    field = arguments[arguments.index('--json') + 1]
                    if field == 'assets':
                        assets = [{'name': path.name} for path in output.iterdir()]
                        assets.append({'name':'validation-evidence.zip'})
                        return subprocess.CompletedProcess(arguments, 0, json.dumps({'assets':assets}), '')
                    return subprocess.CompletedProcess(arguments, 0, json.dumps({'isDraft':True,'assets':([{'name':'build.log'}] if stale_before_upload else [])}), '')
                return subprocess.CompletedProcess(arguments, 0, '', '')
            with self.subTest(stale_before_upload=stale_before_upload), self.assertRaises(ValueError):
                publish('owner/repo', manifest, output, notes, run)
            self.assertFalse(any('--draft=false' in call for call in calls))
            if stale_before_upload:
                self.assertFalse(any(call[2]=='upload' for call in calls))


if __name__ == '__main__':
    unittest.main()
