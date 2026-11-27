"""Manual candidate build defaults, version preflight, and artifact integrity."""
import hashlib
import json
import re
from pathlib import Path
import tempfile
import unittest
import zipfile
from unittest.mock import Mock

import workflow_build as build
from publish_release import TARGETS, digest
from release_assets import validation_directory
import test_publish_release as fixtures

REVISION = fixtures.REVISION


def environment(**changes):
    result = {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'workflow_dispatch',
              'GITHUB_SHA': REVISION, 'GITHUB_REF': 'refs/heads/main', 'GITHUB_REPOSITORY': 'owner/repo',
              'GH_TOKEN': 'fixture-token'}
    result.update(changes)
    return result


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
                'prerelease': False})


    def test_bad_event_version_tag_and_sha_fail_before_building(self):
        for changes in [{'GITHUB_EVENT_NAME': 'pull_request'},
                        {'GITHUB_EVENT_NAME': 'push'}, {'GITHUB_REF': 'refs/heads/other'},
                        {'GITHUB_SHA': 'short'}]:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                build.build_options(environment(**changes), self.root)

    def test_preflight_rejects_published_versions_and_existing_tags(self):
        api = Mock()
        api.get.return_value = None
        build.check_available(api, self.options)
        for response in ({'draft': False}, {'draft': True}, {'object': {}}):
            api.get.return_value = response
            with self.assertRaisesRegex(ValueError, 'prepare_release.py'):
                build.check_available(api, self.options)

    def test_build_only_creates_four_downloads_and_summary_without_github_calls(self):
        manifest, notes = self.assemble()
        self.assertEqual(len(list(self.assets.iterdir())), 4)
        summary = self.root / 'summary.md'
        build.write_summary(self.options, environment(
            GITHUB_STEP_SUMMARY=str(summary), GITHUB_RUN_ID='123', BUILD_ARTIFACT_URL='https://github.com/build-artifact'))
        text = summary.read_text()
        self.assertIn('[Download the validated build](https://github.com/build-artifact)', text)
        self.assertIn(REVISION, text)
        self.assertIn('Build run ID: **123**', text)
        self.assertIn('Publish release candidate', text)
        with tempfile.TemporaryDirectory() as directory:
            verified, _ = build.verify_saved_assets(self.assets, self.options, Path(directory))
        self.assertEqual(verified, manifest)

    def test_native_matrix_and_release_targets_match_the_three_prebuilt_platforms(self):
        expected = {'x86_64-unknown-linux-gnu', 'aarch64-apple-darwin', 'x86_64-pc-windows-msvc'}
        workflow = (Path(__file__).resolve().parents[1] / '.github/workflows/native-validation.yml').read_text()
        targets = re.findall(r'^            target: (\S+)$', workflow, re.M)
        self.assertEqual(set(TARGETS), expected)
        self.assertEqual(len(TARGETS), 3)
        self.assertEqual(set(targets), expected)
        self.assertEqual(len(targets), 3)

    def test_modified_download_or_extra_file_is_rejected_before_any_api_access(self):
        options = self.options
        manifest, _ = self.assemble(options)
        archive = self.assets / manifest['packages'][0]['asset']
        original = archive.read_bytes()
        for reason in ('tampered', 'extra', 'wrong-commit'):
            if reason == 'tampered':
                archive.write_bytes(original + b'changed')
            elif reason == 'extra':
                (self.assets / 'unexpected.txt').touch()
            chosen = {**options, 'source_revision': 'b' * 40} if reason == 'wrong-commit' else options
            with self.subTest(reason=reason), self.assertRaises(ValueError):
                with tempfile.TemporaryDirectory() as directory:
                    build.verify_saved_assets(self.assets, chosen, Path(directory))
            archive.write_bytes(original)
            (self.assets / 'unexpected.txt').unlink(missing_ok=True)

    def test_failed_native_record_is_rejected_even_when_every_checksum_is_updated(self):
        self.assemble()
        evidence = validation_directory(self.assets)
        path = evidence / ('validation-' + TARGETS[0] + '.json')
        record = json.loads(path.read_bytes())
        record['status'] = 'failed'
        path.write_bytes(json.dumps(record).encode())
        manifest_path = evidence / 'release-manifest.json'
        manifest = json.loads(manifest_path.read_bytes())
        manifest['files'][path.name] = digest(path)
        manifest_path.write_bytes(json.dumps(manifest).encode())
        (self.assets / 'SHA256SUMS').write_bytes(''.join(
            f'{digest(item)}  {item.name}\r\n' for item in sorted(self.assets.iterdir())
            if item.name != 'SHA256SUMS').encode())
        with tempfile.TemporaryDirectory() as directory, self.assertRaisesRegex(ValueError, 'did not pass'):
            build.verify_saved_assets(self.assets, self.options, Path(directory))


if __name__ == '__main__':
    unittest.main()
