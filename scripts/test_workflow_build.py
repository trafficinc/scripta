"""Manual build defaults, saved artifact verification, and optional publication."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
import zipfile
from unittest.mock import Mock

import workflow_build as build
from publish_release import digest
import test_publish_release as fixtures

REVISION = fixtures.REVISION


def environment(**changes):
    result = {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'workflow_dispatch',
              'GITHUB_SHA': REVISION, 'GITHUB_REPOSITORY': 'owner/repo',
              'GH_TOKEN': 'fixture-token'}
    result.update(changes)
    return result


class BuildAPI:
    def __init__(self):
        self.responses = {}
        self.posts = []

    def get(self, path, missing=False):
        if path in self.responses:
            return copy.deepcopy(self.responses[path])
        if missing:
            return None
        raise AssertionError('Unexpected API request: ' + path)

    def post(self, path, data):
        self.posts.append((path, data))
        self.responses['/git/ref/' + data['ref'].removeprefix('refs/')] = {
            'object': {'type': 'commit', 'sha': data['sha']}}


class WorkflowBuildTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / 'Cargo.toml').write_bytes(b'[workspace.package]\nversion = "0.1.0"\n')
        self.options = build.build_options(environment(), self.root)
        self.assets = self.root / 'assets'

    def assemble(self, options=None):
        fixtures.ReleaseTests().fixture(self.root / 'inputs')
        return build.assemble(self.root / 'inputs', self.assets, options or self.options)

    def test_ui_defaults_read_committed_version_without_helper_inputs_lf_and_crlf(self):
        for newline in (b'\n', b'\r\n'):
            (self.root / 'Cargo.toml').write_bytes(
                newline.join([b'[workspace.package]', b'version = "0.1.0"', b'']))
            self.assertEqual(build.build_options(environment(), self.root), {
                'version': '0.1.0', 'tag': 'v0.1.0', 'source_revision': REVISION,
                'prerelease': False, 'create_release': False})

    def test_helper_sha_assertion_is_optional_but_enforced_when_present(self):
        self.assertEqual(build.build_options(environment(BUILD_EXPECTED_REVISION=REVISION), self.root),
                         self.options)
        with self.assertRaisesRegex(ValueError, 'expected revision'):
            build.build_options(environment(BUILD_EXPECTED_REVISION='b' * 40), self.root)

    def test_manual_release_and_tag_push_preserve_release_classification(self):
        for tag, prerelease in [('v0.1.0', False), ('v0.1.0-rc.1', True), ('validation/build', True)]:
            for event in ('push', 'workflow_dispatch'):
                env = environment(GITHUB_EVENT_NAME=event, GITHUB_REF_TYPE='tag',
                                  GITHUB_REF_NAME=tag, BUILD_RELEASE_TAG=tag,
                                  BUILD_CREATE_RELEASE='true')
                options = build.build_options(env, self.root)
                self.assertEqual((options['tag'], options['prerelease'], options['create_release']),
                                 (tag, prerelease, True))

    def test_bad_event_version_tag_and_sha_fail_before_building(self):
        for changes in [{'GITHUB_EVENT_NAME': 'pull_request'},
                        {'GITHUB_EVENT_NAME': 'push', 'GITHUB_REF_TYPE': 'branch'},
                        {'BUILD_CREATE_RELEASE': 'yes'}, {'GITHUB_SHA': 'short'},
                        {'BUILD_RELEASE_TAG': 'v0.2.0'}, {'BUILD_RELEASE_TAG': 'bad tag'},
                        {'BUILD_RELEASE_TAG': 'bad\nref'}, {'BUILD_RELEASE_TAG': '../bad'}]:
            with self.subTest(changes=changes), self.assertRaises((ValueError, subprocess.CalledProcessError)):
                build.build_options(environment(**changes), self.root)

    def test_build_only_creates_six_downloads_and_summary_without_github_calls(self):
        manifest, notes = self.assemble()
        self.assertEqual(len(list(self.assets.iterdir())), 6)
        summary = self.root / 'summary.md'
        build.write_summary(self.options, environment(
            GITHUB_STEP_SUMMARY=str(summary), BUILD_ARTIFACT_URL='https://github.com/build-artifact'))
        text = summary.read_text()
        self.assertIn('[Download the validated build](https://github.com/build-artifact)', text)
        self.assertIn(REVISION, text)
        self.assertIn('Build only; no tag or GitHub Release was created.', text)
        with tempfile.TemporaryDirectory() as directory:
            verified, _ = build.verify_saved_assets(self.assets, self.options, Path(directory))
        self.assertEqual(verified, manifest)
        api = BuildAPI()
        publisher = Mock()
        with self.assertRaisesRegex(ValueError, 'authorized'):
            build.publish_saved(self.assets, self.options, environment(), api, publisher)
        self.assertEqual(api.posts, [])
        publisher.assert_not_called()

    def test_manual_release_tags_exact_commit_and_publishes_original_six_files(self):
        options = {**self.options, 'create_release': True}
        self.assemble(options)
        before = {path.name: path.read_bytes() for path in self.assets.iterdir()}
        api = BuildAPI()
        extra = 'User notes\r\nSecond line: `literal` $(no-shell-execution)'
        def publisher(repository, manifest, assets, notes):
            self.assertEqual(repository, 'owner/repo')
            self.assertEqual(assets, self.assets)
            self.assertEqual(manifest['source_revision'], REVISION)
            self.assertEqual(before, {path.name: path.read_bytes() for path in assets.iterdir()})
            self.assertTrue(notes.read_bytes().startswith(extra.replace('\r\n', '\n').encode()))
            self.assertNotIn(b'\r', notes.read_bytes())
            return 'published'
        self.assertEqual(build.publish_saved(self.assets, options,
                         environment(BUILD_RELEASE_NOTES=extra), api, publisher), 'published')
        self.assertEqual(api.posts, [('/git/refs', {'ref': 'refs/tags/v0.1.0', 'sha': REVISION})])

    def test_published_release_and_different_existing_tag_are_never_replaced(self):
        options = {**self.options, 'create_release': True}
        self.assemble(options)
        for reason in ('published', 'other-tag', 'missing-pushed-tag'):
            api = BuildAPI()
            env = environment()
            if reason == 'published':
                api.responses['/releases/tags/v0.1.0'] = {'draft': False}
            elif reason == 'other-tag':
                api.responses['/git/ref/tags/v0.1.0'] = {'object': {'type': 'commit', 'sha': 'b' * 40}}
            else:
                env['GITHUB_EVENT_NAME'] = 'push'
            publisher = Mock()
            with self.subTest(reason=reason), self.assertRaises(ValueError):
                build.publish_saved(self.assets, options, env, api, publisher)
            self.assertEqual(api.posts, [])
            publisher.assert_not_called()

    def test_existing_matching_tag_can_resume_a_draft_without_rebuilding_or_moving_tag(self):
        options = {**self.options, 'create_release': True}
        self.assemble(options)
        api = BuildAPI()
        api.responses['/releases/tags/v0.1.0'] = {'draft': True}
        api.responses['/git/ref/tags/v0.1.0'] = {'object': {'type': 'commit', 'sha': REVISION}}
        publisher = Mock(return_value='published')
        for event in ('push', 'workflow_dispatch'):
            self.assertEqual(build.publish_saved(self.assets, options,
                             environment(GITHUB_EVENT_NAME=event), api, publisher), 'published')
        self.assertEqual(api.posts, [])
        self.assertEqual(publisher.call_count, 2)

    def test_modified_download_or_extra_file_is_rejected_before_any_api_access(self):
        options = {**self.options, 'create_release': True}
        manifest, _ = self.assemble(options)
        archive = self.assets / manifest['packages'][0]['asset']
        original = archive.read_bytes()
        for reason in ('tampered', 'extra', 'wrong-commit'):
            if reason == 'tampered':
                archive.write_bytes(original + b'changed')
            elif reason == 'extra':
                (self.assets / 'unexpected.txt').touch()
            api = Mock()
            publisher = Mock()
            chosen = {**options, 'source_revision': 'b' * 40} if reason == 'wrong-commit' else options
            with self.subTest(reason=reason), self.assertRaises(ValueError):
                build.publish_saved(self.assets, chosen, environment(), api, publisher)
            api.assert_not_called()
            self.assertEqual(api.mock_calls, [])
            publisher.assert_not_called()
            archive.write_bytes(original)
            (self.assets / 'unexpected.txt').unlink(missing_ok=True)

    def test_failed_native_record_is_rejected_even_when_every_checksum_is_updated(self):
        self.assemble()
        path = self.assets / 'validation-evidence.zip'
        with zipfile.ZipFile(path) as bundle:
            entries = {name: bundle.read(name) for name in bundle.namelist()}
        native = next(name for name in entries if name.startswith('validation-') and name.endswith('.json'))
        record = json.loads(entries[native])
        record['status'] = 'failed'
        entries[native] = json.dumps(record).encode()
        manifest = json.loads(entries['release-manifest.json'])
        manifest['files'][native] = hashlib.sha256(entries[native]).hexdigest()
        entries['release-manifest.json'] = json.dumps(manifest).encode()
        with zipfile.ZipFile(path, 'w') as bundle:
            for name, content in entries.items():
                bundle.writestr(name, content)
        (self.assets / 'SHA256SUMS').write_bytes(''.join(
            f'{digest(item)}  {item.name}\r\n' for item in sorted(self.assets.iterdir())
            if item.name != 'SHA256SUMS').encode())
        with tempfile.TemporaryDirectory() as directory, self.assertRaisesRegex(ValueError, 'did not pass'):
            build.verify_saved_assets(self.assets, self.options, Path(directory))


if __name__ == '__main__':
    unittest.main()
