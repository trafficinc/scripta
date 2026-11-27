"""Release staging and publication tests without credentials or network access."""
import json
import io
import tarfile
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import Mock, patch

from publish_release import SUITES, TARGETS, digest, prepare, publish, release_kind

from release_assets import inspect_package, validation_directory

REVISION = 'a' * 40


class ReleaseTests(unittest.TestCase):
    def fixture(self, root, version='0.1.0'):
        for target in TARGETS:
            directory = root / ('native-validation-' + target + '-1')
            (directory / 'logs').mkdir(parents=True)
            (directory / 'logs/check.log').write_bytes(b'passed\n')
            record = {'target': target, 'version': version, 'source_revision': REVISION,
                      'status': 'passed', 'steps': [{'exit_code': 0}],
                      'tests': {name: {'passed': 1, 'failed': 0, 'ignored': 0} for name in SUITES},
                      'packages': {}}
            for variant in ['standard']:
                extension = '.zip' if 'windows' in target else '.tar.gz'
                name = f'scripta-{version}-{target}{extension}'
                path = directory / variant / name
                path.parent.mkdir()
                primary, alias = ('scripta.exe', 'scr.exe') if 'windows' in target else ('scripta', 'scr')
                files = {primary: (target + variant).encode(), alias: (target + variant).encode(), 'LICENSE': b'MIT', 'THIRD_PARTY_NOTICES.txt': b'notices', 'third-party/test-1/LICENSE': b'license'}
                prefix = f'scripta-{version}-{target}/'
                if 'windows' in target:
                    with zipfile.ZipFile(path, 'w') as archive:
                        for member, data in files.items():
                            archive.writestr(prefix + member, data)
                else:
                    with tarfile.open(path, 'w:gz') as archive:
                        for member, data in files.items():
                            entry = tarfile.TarInfo(prefix + member)
                            entry.size = len(data)
                            entry.mode = 0o755 if member in (primary, alias) else 0o644
                            archive.addfile(entry, io.BytesIO(data))
                details = inspect_package(path, version, target)
                details.update(format=2, extensions=['sqlite'], archive_sha256=digest(path))
                path.with_name(path.name + '.package.json').write_bytes(json.dumps(details).encode())
                checksum = digest(path)
                record['packages'][variant] = {
                    'archive': variant + ('\\' if 'windows' in target else '/') + name,
                    'archive_sha256': checksum, 'binary_sha256': details['binary_sha256'],
                    'offline_package_smoke': 'passed', 'identical_input_repack': 'passed',
                    'dependencies': ['system library'], 'extensions': ['sqlite']}
                smoke = {'target': target, 'version': version, 'extensions': ['sqlite'], 'archive_sha256': checksum,
                         'binary_sha256': details['binary_sha256']}
                path.with_name(name + '.validation.json').write_text(json.dumps(smoke), encoding='utf-8')
            (directory / 'validation.json').write_text(json.dumps(record), encoding='utf-8')
        return root / ('native-validation-' + TARGETS[0] + '-1')

    def test_all_targets_and_variants_get_distinct_assets_and_matching_checksums(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root / 'inputs')
            output = root / 'assets'
            manifest, notes = prepare(root / 'inputs', output, REVISION, 'v0.1.0', '0.1.0')
            self.assertFalse(manifest['prerelease'])
            self.assertEqual(len(manifest['packages']), 3)
            self.assertEqual(len({p['asset'] for p in manifest['packages']}), 3)
            self.assertEqual(len(list(output.iterdir())), 4)
            expected_assets = {p['asset'] for p in manifest['packages']} | {'SHA256SUMS'}
            self.assertEqual({p.name for p in output.iterdir()}, expected_assets)
            evidence = validation_directory(output)
            self.assertEqual(json.loads((evidence / 'release-manifest.json').read_bytes()), manifest)
            for package in manifest['packages']:
                self.assertEqual(digest(output / package['asset']), package['archive_sha256'])
                smoke = json.loads((evidence / (package['asset'] + '.validation.json')).read_bytes())
                self.assertEqual(smoke['archive_sha256'], package['archive_sha256'])
            self.assertFalse((output / 'validation-evidence.zip').exists())
            checksums = (output / 'SHA256SUMS').read_text().splitlines()
            self.assertEqual(len(checksums), 3)
            for line in checksums:
                checksum, name = line.split('  ', 1)
                self.assertEqual(digest(output / name), checksum)
            self.assertIn('no Rust installation', notes.read_text())
            self.assertTrue((output / 'scripta-0.1.0-x86_64-pc-windows-msvc.zip').is_file())

    def test_generated_text_uses_lf_under_windows_newline_translation(self):
        def windows_write_text(path, data, encoding=None, errors=None):
            with path.open('w', encoding=encoding, errors=errors, newline='\r\n') as stream:
                return stream.write(data)

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(Path, 'write_text', windows_write_text):
                target = self.fixture(root / 'inputs')
                # Native logs are copied verbatim, even when they contain CRLF.
                (target / 'logs/check.log').write_bytes(b'passed\r\n')
                output = root / 'assets'
                manifest, notes = prepare(root / 'inputs', output, REVISION, 'v0.1.0', '0.1.0')
            evidence = validation_directory(output)
            for package in manifest['packages']:
                self.assertEqual((evidence / (package['asset'] + '.sha256')).read_bytes(),
                                 (package['archive_sha256'] + '  ' + package['asset'] + '\n').encode())
            self.assertEqual((evidence / 'release-manifest.json').read_bytes(),
                             (json.dumps(manifest, indent=2) + '\n').encode())
            self.assertEqual((evidence / f"logs/{TARGETS[0]}/check.log").read_bytes(), b'passed\r\n')
            checksums = ''.join(f'{digest(path)}  {path.name}\n' for path in sorted(output.iterdir())
                                if path.name != 'SHA256SUMS')
            self.assertEqual((output / 'SHA256SUMS').read_bytes(), checksums.encode('utf-8'))
            self.assertNotIn(b'\r', notes.read_bytes())
            self.assertTrue(notes.read_bytes().endswith(b'\n'))

    def test_failed_missing_mismatched_or_tampered_inputs_stop_before_staging(self):
        for failure in ['target', 'revision', 'version', 'failed', 'ignored', 'steps', 'variant', 'checksum', 'smoke', 'sqlite', 'smoke-sqlite']:
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                target = self.fixture(root / 'inputs')
                path = target / 'validation.json'
                record = json.loads(path.read_text())
                if failure == 'target': path.unlink()
                elif failure == 'revision': record['source_revision'] = 'c' * 40
                elif failure == 'version': record['version'] = '0.2.0'
                elif failure in ['failed', 'ignored']: record['tests'][next(iter(SUITES))][failure] = 1
                elif failure == 'steps': record['steps'][0]['exit_code'] = 1
                elif failure == 'variant': del record['packages']['standard']
                elif failure == 'checksum': record['packages']['standard']['archive_sha256'] = '0' * 64
                elif failure == 'sqlite': record['packages']['standard']['extensions'] = []
                elif failure == 'smoke-sqlite':
                    archive = target / record['packages']['standard']['archive']
                    smoke_path = archive.with_name(archive.name + '.validation.json')
                    smoke_record = json.loads(smoke_path.read_text())
                    smoke_record['extensions'] = []
                    smoke_path.write_text(json.dumps(smoke_record))
                elif failure == 'smoke':
                    archive = target / record['packages']['standard']['archive']
                    archive.with_name(archive.name + '.validation.json').write_text('{}')
                if failure != 'target': path.write_text(json.dumps(record))
                with self.assertRaises(ValueError):
                    prepare(root / 'inputs', root / 'assets', REVISION, 'v0.1.0', '0.1.0')
                self.assertFalse((root / 'assets').exists())

    def test_linux_arm64_input_is_rejected_before_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = self.fixture(root / 'inputs')
            arm = root / 'inputs' / 'native-validation-aarch64-unknown-linux-gnu'
            shutil.copytree(first, arm)
            record = json.loads((arm / 'validation.json').read_text())
            record['target'] = 'aarch64-unknown-linux-gnu'
            (arm / 'validation.json').write_text(json.dumps(record))
            with self.assertRaises(ValueError):
                prepare(root / 'inputs', root / 'assets', REVISION, 'v0.1.0', '0.1.0')
            self.assertFalse((root / 'assets').exists())

    def test_version_and_prerelease_classification(self):
        self.assertFalse(release_kind('v0.1.0', '0.1.0'))
        for tag in ['v0.1.0-validation.1', 'v0.1.0-rc.1', 'nightly', 'test/build']:
            self.assertTrue(release_kind(tag, '0.1.0'))
        self.assertTrue(release_kind('v0.1.0-rc.1', '0.1.0-rc.1'))
        for tag in ['v0.2.0', '--bad', '']:
            with self.assertRaises(ValueError): release_kind(tag, '0.1.0')

    def test_upload_failure_leaves_release_as_draft_and_published_assets_are_not_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root / 'inputs')
            output = root / 'assets'
            manifest, _ = prepare(root / 'inputs', output, REVISION, 'v0.1.0', '0.1.0')
            calls = []
            def run(arguments, **kwargs):
                calls.append(arguments)
                operation = arguments[2]
                if operation == 'view': return subprocess.CompletedProcess(arguments, 1, '', '')
                if operation == 'upload': raise subprocess.CalledProcessError(1, arguments)
                return subprocess.CompletedProcess(arguments, 0, '', '')
            with self.assertRaises(subprocess.CalledProcessError):
                publish('owner/repo', manifest, output, output / 'notes.md', run)
            self.assertTrue(any('--draft' in call for call in calls))
            self.assertFalse(any('--draft=false' in call for call in calls))
            already_published = Mock(return_value=subprocess.CompletedProcess([], 0, '{"isDraft": false}', ''))
            with self.assertRaises(ValueError):
                publish('owner/repo', manifest, output, output / 'notes.md', already_published)
            self.assertEqual(already_published.call_count, 1)

    def test_prerelease_is_published_only_after_upload_and_draft_can_resume(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root / 'inputs')
            output = root / 'assets'
            manifest, _ = prepare(root / 'inputs', output, REVISION, 'v0.1.0-test', '0.1.0')
            for draft_exists in [False, True]:
                calls = []
                def run(arguments, **kwargs):
                    calls.append(arguments)
                    if arguments[2] == 'view':
                        if arguments[arguments.index('--json') + 1] == 'assets':
                            return subprocess.CompletedProcess(arguments, 0, json.dumps({'assets':[{'name':p.name} for p in output.iterdir()]}), '')
                        return subprocess.CompletedProcess(arguments, 0 if draft_exists else 1, '{"isDraft": true}', '')
                    return subprocess.CompletedProcess(arguments, 0, '', '')
                publish('owner/repo', manifest,
                        output, output / 'notes.md', run)
                upload = next(i for i, call in enumerate(calls) if call[2] == 'upload')
                publication = next(i for i, call in enumerate(calls) if '--draft=false' in call)
                self.assertLess(upload, publication)
                self.assertIn('--prerelease=true', calls[publication])
                self.assertIn('--latest=false', calls[publication])


if __name__ == '__main__':
    unittest.main()
