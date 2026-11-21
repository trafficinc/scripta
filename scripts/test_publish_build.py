"""Saved build selection, provenance, retagging, and publication gates."""
import base64
import copy
from pathlib import Path
import tempfile
import unittest
import zipfile
from unittest.mock import Mock

import publish_build as saved
import test_publish_release as fixtures
from publish_release import digest, prepare

REVISION = fixtures.REVISION
REPOSITORY = 'owner/repo'
ENV = {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'workflow_dispatch',
       'GITHUB_REF': 'refs/heads/main', 'PROMOTION_DEFAULT_BRANCH': 'main',
       'GH_TOKEN': 'fixture-token', 'GITHUB_REPOSITORY': REPOSITORY}


class FakeAPI:
    def __init__(self):
        self.run = {'id': 123, 'path': saved.WORKFLOW, 'event': 'workflow_dispatch',
                    'head_sha': REVISION, 'head_branch': 'main', 'status': 'completed',
                    'conclusion': 'success', 'run_attempt': 1,
                    'repository': {'full_name': REPOSITORY},
                    'head_repository': {'full_name': REPOSITORY}}
        self.jobs = [{'name': target, 'status': 'completed', 'conclusion': 'success'}
                     for target in [*saved.TARGETS, 'Assemble validated downloads']]
        self.jobs.append({'name': 'Publish release downloads', 'status': 'completed', 'conclusion': 'skipped'})
        self.artifact = {'id': 456, 'name': 'scripta-build-123-1', 'expired': False,
                         'workflow_run': {'head_sha': REVISION}}
        self.responses = {}
        self.posts = []

    def get(self, path, missing=False):
        if path in self.responses:
            return copy.deepcopy(self.responses[path])
        if path == '/actions/runs/123':
            return copy.deepcopy(self.run)
        if path == '/actions/runs/123/jobs?per_page=100':
            return {'total_count': len(self.jobs), 'jobs': copy.deepcopy(self.jobs)}
        if path == '/actions/workflows/native-validation.yml/runs?branch=main&status=success&per_page=1':
            return {'workflow_runs': [copy.deepcopy(self.run)]}
        if path == '/actions/runs/123/artifacts?per_page=100&page=1':
            return {'artifacts': [copy.deepcopy(self.artifact)]}
        if path == '/contents/Cargo.toml?ref=' + REVISION:
            return {'encoding': 'base64', 'size': 50,
                    'content': base64.b64encode(b'[workspace.package]\r\nversion = "0.1.0"\r\n').decode()}
        if missing:
            return None
        raise AssertionError('Unexpected API request: ' + path)

    def post(self, path, data):
        self.posts.append((path, data))
        if path == '/git/tags':
            self.responses['/git/tags/' + 'c' * 40] = {
                'tag': data['tag'], 'message': data['message'],
                'object': {'type': 'commit', 'sha': data['object']}}
            return {'sha': 'c' * 40}
        if path == '/git/refs':
            self.responses['/git/ref/' + data['ref'].removeprefix('refs/')] = {
                'object': {'type': 'tag', 'sha': data['sha']}}
            return {}
        raise AssertionError('Unexpected write: ' + path)


class PublishBuildTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.api = FakeAPI()
        self.source = self.root / 'source'
        self.output = self.root / 'output'

    def fixture(self, tag='v0.1.0-rc.1'):
        fixtures.ReleaseTests().fixture(self.root / 'native')
        prepare(self.root / 'native', self.source, REVISION, 'v0.1.0', '0.1.0')
        selection = saved.select_build(self.api, REPOSITORY, tag)
        manifest, notes = saved.prepare_saved(self.api, selection, self.source, self.output)
        return selection, manifest, notes

    def test_latest_main_or_explicit_run_selects_artifact_and_source_version(self):
        for run_id in (None, 123):
            selection = saved.select_build(self.api, REPOSITORY, 'v0.1.0', run_id)
            self.assertEqual(selection['artifact_id'], 456)
            self.assertEqual(selection['source_revision'], REVISION)
            self.assertEqual(selection['version'], '0.1.0')
        self.api.run['head_branch'] = 'reviewed-branch'
        with self.assertRaisesRegex(ValueError, 'main'):
            saved.select_build(self.api, REPOSITORY, 'v0.1.0')
        self.assertEqual(saved.select_build(self.api, REPOSITORY, 'v0.1.0', 123)['run_id'], 123)

    def test_wrong_version_or_unsuccessful_foreign_or_wrong_workflow_runs_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'workspace version'):
            saved.select_build(self.api, REPOSITORY, 'v0.2.0')
        for key, value in [('conclusion', 'failure'), ('path', 'other.yml'),
                           ('event', 'pull_request'), ('status', 'in_progress'),
                           ('repository', {'full_name': 'other/repo'}),
                           ('head_repository', {'full_name': 'fork/repo'})]:
            original = self.api.run[key]
            self.api.run[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                saved.select_build(self.api, REPOSITORY, 'v0.1.0', 123)
            self.api.run[key] = original
        self.assertEqual(self.api.posts, [])

    def test_all_native_jobs_and_assembly_are_required(self):
        for index in range(5):
            self.api.jobs[index]['conclusion'] = 'failure'
            with self.subTest(index=index), self.assertRaises(ValueError):
                saved.select_build(self.api, REPOSITORY, 'v0.1.0', 123)
            self.api.jobs[index]['conclusion'] = 'success'
        self.api.jobs.pop(0)
        with self.assertRaises(ValueError):
            saved.select_build(self.api, REPOSITORY, 'v0.1.0', 123)

    def test_missing_expired_and_wrong_commit_artifacts_fail_without_writes(self):
        for key, value in [('expired', True), ('name', 'native-validation-linux'),
                           ('workflow_run', {'head_sha': 'b' * 40})]:
            original = self.api.artifact[key]
            self.api.artifact[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                saved.select_build(self.api, REPOSITORY, 'v0.1.0')
            self.api.artifact[key] = original
        self.assertEqual(self.api.posts, [])

    def test_packages_and_receipts_stay_unchanged_and_metadata_records_new_tag_and_build(self):
        selection, manifest, notes = self.fixture()
        self.assertEqual(manifest['tag'], 'v0.1.0-rc.1')
        self.assertTrue(manifest['prerelease'])
        self.assertEqual(manifest['built_from']['artifact_id'], 456)
        self.assertEqual(manifest['built_from']['evidence_sha256'], digest(self.source / 'validation-evidence.zip'))
        self.assertEqual(len(list(self.output.iterdir())), 6)
        for package in manifest['packages']:
            self.assertEqual((self.source / package['asset']).read_bytes(),
                             (self.output / package['asset']).read_bytes())
        with zipfile.ZipFile(self.source / 'validation-evidence.zip') as before, zipfile.ZipFile(self.output / 'validation-evidence.zip') as after:
            for name in before.namelist():
                if name != 'release-manifest.json':
                    self.assertEqual(before.read(name), after.read(name))
        self.assertIn('Only release metadata', notes.read_text())
        for line in (self.output / 'SHA256SUMS').read_text().splitlines():
            checksum, name = line.split('  ')
            self.assertEqual(checksum, digest(self.output / name))
        self.assertEqual(self.api.posts, [])

    def test_tags_build_commit_and_resumes_own_draft_without_replacing_published_release(self):
        selection, manifest, notes = self.fixture('v0.1.0')
        publisher = Mock(return_value='published')
        self.assertEqual(saved.publish_build(self.api, selection, manifest, self.output, notes, ENV, publisher), 'published')
        self.assertEqual(self.api.posts[0][1]['object'], REVISION)
        self.assertEqual(self.api.posts[1][1]['ref'], 'refs/tags/v0.1.0')
        self.api.posts.clear()
        saved.publish_build(self.api, selection, manifest, self.output, notes, ENV, publisher)
        self.assertEqual(self.api.posts, [])
        self.api.responses['/releases/tags/v0.1.0'] = {'draft': False}
        with self.assertRaisesRegex(ValueError, 'already published'):
            saved.publish_build(self.api, selection, manifest, self.output, notes, ENV, publisher)
        self.assertEqual(publisher.call_count, 2)

    def test_upload_failure_propagates_and_tag_is_preserved_for_retry(self):
        selection, manifest, notes = self.fixture('v0.1.0')
        publisher = Mock(side_effect=RuntimeError('upload failed'))
        with self.assertRaisesRegex(RuntimeError, 'upload failed'):
            saved.publish_build(self.api, selection, manifest, self.output, notes, ENV, publisher)
        self.assertEqual(len(self.api.posts), 2)
        self.api.posts.clear()
        saved.publish_build(self.api, selection, manifest, self.output, notes, ENV, Mock())
        self.assertEqual(self.api.posts, [])

    def test_existing_foreign_tag_invalid_dispatch_and_retried_run_prevent_publication(self):
        selection, manifest, notes = self.fixture('v0.1.0')
        publisher = Mock()
        self.api.responses['/git/ref/tags/v0.1.0'] = {'object': {'type': 'commit', 'sha': REVISION}}
        with self.assertRaisesRegex(ValueError, 'already exists'):
            saved.publish_build(self.api, selection, manifest, self.output, notes, ENV, publisher)
        self.api.responses.clear()
        for key, value in [('GITHUB_REF', 'refs/heads/other'), ('GH_TOKEN', ''), ('GITHUB_EVENT_NAME', 'push')]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                saved.publish_build(self.api, selection, manifest, self.output, notes, {**ENV, key: value}, publisher)
        self.api.run['run_attempt'] = 2
        with self.assertRaisesRegex(ValueError, 'retried'):
            saved.publish_build(self.api, selection, manifest, self.output, notes, ENV, publisher)
        self.assertEqual(self.api.posts, [])
        publisher.assert_not_called()

    def test_tampered_package_fails_before_tag_creation(self):
        fixtures.ReleaseTests().fixture(self.root / 'native')
        manifest, _ = prepare(self.root / 'native', self.source, REVISION, 'v0.1.0', '0.1.0')
        package = self.source / manifest['packages'][0]['asset']
        package.write_bytes(package.read_bytes() + b'tampered')
        selection = saved.select_build(self.api, REPOSITORY, 'v0.1.0')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            saved.prepare_saved(self.api, selection, self.source, self.output)
        self.assertEqual(self.api.posts, [])


if __name__ == '__main__':
    unittest.main()
