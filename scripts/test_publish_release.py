"""Release staging and publication tests without credentials or network access."""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock

from publish_release import SUITES, TARGETS, digest, prepare, publish, release_kind

REVISION = 'a' * 40


class ReleaseTests(unittest.TestCase):
    def fixture(self, root):
        for target in TARGETS:
            directory = root / ('native-validation-' + target + '-1')
            (directory / 'logs').mkdir(parents=True)
            (directory / 'logs/check.log').write_text('passed\n', encoding='utf-8')
            record = {'target': target, 'version': '0.1.0', 'source_revision': REVISION,
                      'status': 'passed', 'steps': [{'exit_code': 0}],
                      'tests': {name: {'passed': 1, 'failed': 0, 'ignored': 0} for name in SUITES},
                      'packages': {}}
            for variant in ['base', 'sqlite']:
                extension = '.zip' if 'windows' in target else '.tar.gz'
                name = f'scripta-0.1.0-{target}{extension}'
                path = directory / variant / name
                path.parent.mkdir()
                path.write_bytes((target + variant).encode())
                checksum = digest(path)
                record['packages'][variant] = {
                    'archive': variant + ('\\' if 'windows' in target else '/') + name,
                    'archive_sha256': checksum, 'binary_sha256': 'b' * 64,
                    'offline_package_smoke': 'passed', 'identical_input_repack': 'passed',
                    'dependencies': ['system library']}
                smoke = {'target': target, 'version': '0.1.0', 'archive_sha256': checksum,
                         'binary_sha256': 'b' * 64}
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
            self.assertEqual(len(manifest['packages']), 8)
            self.assertEqual(len({p['asset'] for p in manifest['packages']}), 8)
            for package in manifest['packages']:
                self.assertEqual(digest(output / package['asset']), package['archive_sha256'])
                self.assertEqual((output / (package['asset'] + '.sha256')).read_text(),
                                 package['archive_sha256'] + '  ' + package['asset'] + '\n')
            self.assertEqual(len((output / 'SHA256SUMS').read_text().splitlines()), 8)
            self.assertIn('no Rust installation', notes.read_text())
            self.assertTrue((output / 'scripta-0.1.0-x86_64-pc-windows-msvc-sqlite.zip').is_file())

    def test_failed_missing_mismatched_or_tampered_inputs_stop_before_staging(self):
        for failure in ['target', 'revision', 'version', 'failed', 'ignored', 'steps', 'variant', 'checksum', 'smoke']:
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
                elif failure == 'variant': del record['packages']['sqlite']
                elif failure == 'checksum': record['packages']['base']['archive_sha256'] = '0' * 64
                elif failure == 'smoke':
                    archive = target / record['packages']['base']['archive']
                    archive.with_name(archive.name + '.validation.json').write_text('{}')
                if failure != 'target': path.write_text(json.dumps(record))
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
            output = Path(directory)
            (output / 'asset.zip').write_bytes(b'asset')
            manifest = {'tag': 'v0.1.0', 'source_revision': REVISION, 'prerelease': False}
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
            output = Path(directory)
            (output / 'asset.zip').write_bytes(b'asset')
            for draft_exists in [False, True]:
                calls = []
                def run(arguments, **kwargs):
                    calls.append(arguments)
                    if arguments[2] == 'view':
                        return subprocess.CompletedProcess(arguments, 0 if draft_exists else 1, '{"isDraft": true}', '')
                    return subprocess.CompletedProcess(arguments, 0, '', '')
                publish('owner/repo', {'tag': 'v0.1.0-test', 'source_revision': REVISION, 'prerelease': True},
                        output, output / 'notes.md', run)
                upload = next(i for i, call in enumerate(calls) if call[2] == 'upload')
                publication = next(i for i, call in enumerate(calls) if '--draft=false' in call)
                self.assertLess(upload, publication)
                self.assertIn('--prerelease=true', calls[publication])
                self.assertIn('--latest=false', calls[publication])


if __name__ == '__main__':
    unittest.main()
