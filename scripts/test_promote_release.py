"""Promotion integrity, immutability, and no-build tests; no network/credentials."""
import base64
import copy
import hashlib
import json
import os
import shutil
import stat
import tempfile
import unittest
import urllib.request
import warnings
import zipfile
from pathlib import Path
from unittest.mock import Mock, patch

import promote_release as promotion
import test_publish_release as fixtures
from publish_release import TARGETS, digest, prepare

REVISION = fixtures.REVISION
SOURCE_TAG = 'v0.1.0-validation.20261115.5'
STABLE_TAG = 'v0.1.0'
REPOSITORY = 'owner/repo'
RUN_ID = 123
SOURCE_OBJECT = 'b' * 40
CREATED_OBJECT = 'c' * 40
ENV = {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'workflow_dispatch',
       'GITHUB_REF': 'refs/heads/main', 'PROMOTION_DEFAULT_BRANCH': 'main',
       'GH_TOKEN': 'fixture-token', 'GITHUB_SHA': 'd' * 40}


class FakeAPI:
    def __init__(self, root):
        fixtures.ReleaseTests().fixture(root / 'native')
        self.source = root / 'candidate'
        prepare(root / 'native', self.source, REVISION, SOURCE_TAG, '0.1.0')
        self.assets = [{'id': index, 'name': path.name, 'size': path.stat().st_size}
                       for index, path in enumerate(sorted(self.source.iterdir()), 1)]
        self.responses = {
            promotion.tag_path(SOURCE_TAG): {'object': {'type': 'tag', 'sha': SOURCE_OBJECT}},
            '/git/tags/' + SOURCE_OBJECT: {'object': {'type': 'commit', 'sha': REVISION}},
            '/contents/Cargo.toml?ref=' + REVISION: {
                'encoding': 'base64', 'size': 50,
                'content': base64.b64encode(b'[workspace.package]\nversion = "0.1.0"\n').decode()},
            '/actions/runs/123': {
                'id': RUN_ID, 'event': 'push', 'path': promotion.WORKFLOW,
                'head_sha': REVISION, 'head_branch': SOURCE_TAG,
                'status': 'completed', 'conclusion': 'success', 'run_attempt': 1,
                'repository': {'full_name': REPOSITORY}, 'head_repository': {'full_name': REPOSITORY},
                'html_url': 'https://github.com/owner/repo/actions/runs/123'},
            '/actions/runs/123/jobs?per_page=100': {
                'total_count': 5,
                'jobs': [{'name': name, 'status': 'completed', 'conclusion': 'success'}
                         for name in [*TARGETS, 'Publish release downloads']]},
            '/releases/tags/' + SOURCE_TAG: {
                'tag_name': SOURCE_TAG, 'draft': False, 'prerelease': True,
                'assets': self.assets,
                'html_url': 'https://github.com/owner/repo/releases/tag/' + SOURCE_TAG},
        }
        self.posts = []
        self.downloads = []

    def get(self, path, missing=False):
        if path not in self.responses:
            if missing:
                return None
            raise AssertionError('Unexpected API path: ' + path)
        return copy.deepcopy(self.responses[path])

    def post(self, path, data):
        self.posts.append((path, data))
        if path == '/git/tags':
            self.responses['/git/tags/' + CREATED_OBJECT] = {
                'tag': data['tag'], 'message': data['message'],
                'object': {'type': data['type'], 'sha': data['object'], 'url': 'https://api.github.com/commit'}}
            return {'sha': CREATED_OBJECT}
        if path == '/git/refs':
            key = '/git/ref/' + data['ref'].removeprefix('refs/')
            if key in self.responses:
                raise ValueError('Tag race: ref already exists')
            self.responses[key] = {'object': {'type': 'tag', 'sha': data['sha']}}
            return self.responses[key]
        raise AssertionError('Unexpected write: ' + path)

    def download(self, asset, destination, limit):
        self.downloads.append(asset['name'])
        if asset['size'] > limit:
            raise ValueError('Asset exceeds size limit')
        shutil.copyfile(self.source / asset['name'], destination)

    def refresh_checksums(self):
        paths = [path for path in sorted(self.source.iterdir()) if path.name != 'SHA256SUMS']
        (self.source / 'SHA256SUMS').write_bytes(''.join(f'{digest(path)}  {path.name}\n' for path in paths).encode())
        for asset in self.assets:
            asset['size'] = (self.source / asset['name']).stat().st_size

    def edit_evidence(self, change, rehash_members=False):
        path = self.source / 'validation-evidence.zip'
        with zipfile.ZipFile(path) as bundle:
            entries = {info.filename: bundle.read(info) for info in bundle.infolist()}
        change(entries)
        if rehash_members:
            manifest = json.loads(entries['release-manifest.json'])
            for name, content in entries.items():
                if name != 'release-manifest.json':
                    manifest['files'][name] = hashlib.sha256(content).hexdigest()
            entries['release-manifest.json'] = json.dumps(manifest).encode()
        with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as bundle:
            for name, content in entries.items():
                bundle.writestr(name, content)
        self.refresh_checksums()


class PromotionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.api = FakeAPI(self.root)
        self.output = self.root / 'promoted'

    def prepare(self, release_tag=STABLE_TAG):
        return promotion.prepare_promotion(self.api, REPOSITORY, SOURCE_TAG, RUN_ID, release_tag, self.output)

    def test_packages_and_raw_native_evidence_are_unchanged_but_release_metadata_is_stable(self):
        manifest, notes = self.prepare()
        self.assertEqual(manifest['source_revision'], REVISION)
        self.assertEqual(manifest['tag'], STABLE_TAG)
        self.assertFalse(manifest['prerelease'])
        self.assertEqual(manifest['promoted_from']['run_id'], RUN_ID)
        self.assertEqual(len(list(self.output.iterdir())), 6)
        self.assertEqual(self.api.posts, [])
        for package in manifest['packages']:
            self.assertEqual((self.output / package['asset']).read_bytes(), (self.api.source / package['asset']).read_bytes())
        with zipfile.ZipFile(self.api.source / 'validation-evidence.zip') as old, zipfile.ZipFile(self.output / 'validation-evidence.zip') as new:
            self.assertEqual(json.loads(new.read('release-manifest.json')), manifest)
            for name in old.namelist():
                if name != 'release-manifest.json':
                    self.assertEqual(old.read(name), new.read(name), name)
        for line in (self.output / 'SHA256SUMS').read_text().splitlines():
            checksum, name = line.split('  ', 1)
            self.assertEqual(digest(self.output / name), checksum)
        self.assertIn('promoted byte-for-byte', notes.read_text())

    def test_native_run_identity_and_all_jobs_must_pass_before_download(self):
        run = self.api.responses['/actions/runs/123']
        for key, value in [('head_sha', '0' * 40), ('head_branch', 'other'), ('event', 'workflow_dispatch'),
                           ('status', 'in_progress'), ('conclusion', 'failure'), ('path', 'other.yml'),
                           ('repository', {'full_name': 'other/repo'}), ('head_repository', {'full_name': 'fork/repo'})]:
            with self.subTest(key=key):
                original = run[key]
                run[key] = value
                with self.assertRaises(ValueError):
                    self.prepare()
                run[key] = original
        jobs = self.api.responses['/actions/runs/123/jobs?per_page=100']['jobs']
        jobs[0]['conclusion'] = 'failure'
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertFalse(self.output.exists())
        self.assertEqual(self.api.downloads, [])
        self.assertEqual(self.api.posts, [])

    def test_wrong_version_published_destination_and_missing_assets_stop_before_download(self):
        with self.assertRaises(ValueError):
            self.prepare('v0.2.0')
        self.api.responses['/releases/tags/' + STABLE_TAG] = {'draft': False}
        with self.assertRaises(ValueError):
            self.prepare()
        del self.api.responses['/releases/tags/' + STABLE_TAG]
        self.api.assets.pop()
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertEqual(self.api.downloads, [])
        self.assertEqual(self.api.posts, [])

    def test_tampered_package_is_rejected_without_a_stable_tag(self):
        name = next(iter(promotion.package_names('0.1.0').values()))
        (self.api.source / name).write_bytes(b'tampered')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            self.prepare()
        self.assertEqual(self.api.posts, [])
        self.assertFalse(self.output.exists())

    def test_even_rehashed_evidence_cannot_claim_failed_or_different_source_validation(self):
        def fail_record(entries):
            name = 'validation-' + TARGETS[0] + '.json'
            record = json.loads(entries[name])
            record['tests']['tests-dist-default']['failed'] = 1
            entries[name] = json.dumps(record).encode()
        self.api.edit_evidence(fail_record, rehash_members=True)
        with self.assertRaisesRegex(ValueError, 'Native tests failed'):
            self.prepare()
        self.assertFalse(self.output.exists())
        self.assertEqual(self.api.posts, [])

    def test_zip_traversal_is_rejected(self):
        self.api.edit_evidence(lambda entries: entries.update({'../outside': b'bad'}))
        with self.assertRaisesRegex(ValueError, 'unsafe'):
            self.prepare()
        self.assertFalse((self.root / 'outside').exists())
        self.assertEqual(self.api.posts, [])

    def test_symlink_entries_are_rejected(self):
        path = self.api.source / 'validation-evidence.zip'
        with zipfile.ZipFile(path, 'a') as bundle:
            info = zipfile.ZipInfo('logs/' + TARGETS[0] + '/link.log')
            info.external_attr = (stat.S_IFLNK | 0o777) << 16
            bundle.writestr(info, b'/outside')
        self.api.refresh_checksums()
        with self.assertRaisesRegex(ValueError, 'unsafe'):
            self.prepare()
        self.assertEqual(self.api.posts, [])

    def test_duplicate_zip_entries_are_rejected(self):
        path = self.api.source / 'validation-evidence.zip'
        with warnings.catch_warnings():
            warnings.simplefilter('ignore', UserWarning)
            with zipfile.ZipFile(path, 'a') as bundle:
                bundle.writestr('release-manifest.json', b'{}')
        self.api.refresh_checksums()
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            self.prepare()
        self.assertEqual(self.api.posts, [])

    def test_evidence_size_budget_is_checked_before_extraction(self):
        with patch.object(promotion, 'UNPACKED_LIMIT', 16):
            with self.assertRaisesRegex(ValueError, 'exceeds'):
                self.prepare()
        self.assertEqual(self.api.posts, [])

    def test_manual_promotion_tags_validated_commit_instead_of_workflow_commit(self):
        manifest, notes = self.prepare()
        publisher = Mock(return_value='published')
        self.assertEqual(promotion.promote(self.api, REPOSITORY, manifest, self.output, notes, ENV, publisher), 'published')
        self.assertEqual(self.api.posts[0][1]['object'], REVISION)
        self.assertNotEqual(self.api.posts[0][1]['object'], ENV['GITHUB_SHA'])
        self.assertEqual(self.api.posts[1][1], {'ref': 'refs/tags/' + STABLE_TAG, 'sha': CREATED_OBJECT})
        publisher.assert_called_once_with(REPOSITORY, manifest, self.output, notes)

    def test_existing_different_tag_is_never_moved_or_published(self):
        manifest, notes = self.prepare()
        self.api.responses[promotion.tag_path(STABLE_TAG)] = {'object': {'type': 'commit', 'sha': '0' * 40}}
        publisher = Mock()
        with self.assertRaisesRegex(ValueError, 'Existing stable tag'):
            promotion.promote(self.api, REPOSITORY, manifest, self.output, notes, ENV, publisher)
        self.assertEqual(self.api.posts, [])
        publisher.assert_not_called()

    def test_own_annotated_tag_and_draft_can_resume_but_published_release_cannot(self):
        manifest, notes = self.prepare()
        promotion.ensure_promotion_tag(self.api, manifest)
        self.api.posts.clear()
        self.api.responses['/releases/tags/' + STABLE_TAG] = {'draft': True}
        publisher = Mock(return_value='published')
        promotion.promote(self.api, REPOSITORY, manifest, self.output, notes, ENV, publisher)
        self.assertEqual(self.api.posts, [])
        self.api.responses['/releases/tags/' + STABLE_TAG] = {'draft': False}
        with self.assertRaisesRegex(ValueError, 'published'):
            promotion.promote(self.api, REPOSITORY, manifest, self.output, notes, ENV, publisher)
        self.assertEqual(publisher.call_count, 1)

    def test_source_tag_move_run_retry_or_invalid_dispatch_prevents_tag_creation(self):
        manifest, notes = self.prepare()
        self.api.responses['/git/tags/' + SOURCE_OBJECT]['object']['sha'] = '0' * 40
        with self.assertRaisesRegex(ValueError, 'changed'):
            promotion.promote(self.api, REPOSITORY, manifest, self.output, notes, ENV, Mock())
        self.api.responses['/git/tags/' + SOURCE_OBJECT]['object']['sha'] = REVISION
        self.api.responses['/actions/runs/123']['run_attempt'] = 2
        with self.assertRaisesRegex(ValueError, 'retried'):
            promotion.promote(self.api, REPOSITORY, manifest, self.output, notes, ENV, Mock())
        for key, value in [('GITHUB_EVENT_NAME', 'push'), ('GITHUB_REF', 'refs/heads/other'), ('GH_TOKEN', '')]:
            environment = dict(ENV, **{key: value})
            with self.assertRaises(ValueError):
                promotion.promote(self.api, REPOSITORY, manifest, self.output, notes, environment, Mock())
        self.assertEqual(self.api.posts, [])

    def test_workflow_is_manual_and_uses_builtin_token_without_build_steps(self):
        path = Path(__file__).resolve().parents[1] / '.github/workflows/promote-release.yml'
        workflow = path.read_text()
        self.assertIn('  workflow_dispatch:', workflow)
        self.assertNotIn('  push:', workflow)
        self.assertNotIn('  pull_request:', workflow)
        self.assertIn('GH_TOKEN: ${{ github.token }}', workflow)
        self.assertNotIn('cargo ', workflow)
        self.assertNotIn('rustup ', workflow)
        self.assertIn('--publish', workflow)

    def test_cross_host_download_redirect_does_not_forward_authorization(self):
        request = urllib.request.Request('https://api.github.com/asset', headers={'Authorization': 'Bearer fixture'})
        redirect = promotion.SafeRedirect().redirect_request(request, None, 302, 'Found', {}, 'https://release-assets.githubusercontent.com/file')
        self.assertIsNone(redirect.get_header('Authorization'))
        with self.assertRaises(ValueError):
            promotion.SafeRedirect().redirect_request(request, None, 302, 'Found', {}, 'http://example.com/file')


if __name__ == '__main__':
    unittest.main()
