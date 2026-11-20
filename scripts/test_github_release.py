"""GitHub validation and tag gates without network access or publication."""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from github_release import GitHubRelease, WORKFLOW_PATH, verify_jobs, verify_run
from validate_native import TARGETS

REVISION = 'a' * 40


def record(**changes):
    result = {'head_sha': REVISION, 'event': 'workflow_dispatch', 'path': WORKFLOW_PATH,
              'status': 'completed', 'conclusion': 'success', 'run_attempt': 1,
              'html_url': 'https://github.com/owner/repo/actions/runs/42'}
    result.update(changes)
    return result


class GitHubReleaseTests(unittest.TestCase):
    def fixture(self):
        release = GitHubRelease()
        release.repository = 'owner/repo'
        release.run = Mock(return_value=Mock(returncode=1, stdout=''))
        release.api = Mock(return_value=record())
        release.gh = Mock(return_value=json.dumps({'jobs': [
            {'name': target, 'conclusion': 'success'} for target in TARGETS]}))
        release.git = Mock(return_value='')
        release.clean_revision = Mock(return_value=REVISION)
        return release

    def test_wrong_commit_workflow_or_event_cannot_be_used(self):
        verify_run(record(), REVISION)
        for changes in [{'head_sha': 'b' * 40}, {'event': 'push'}, {'path': 'other.yml'}]:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                verify_run(record(**changes), REVISION)

    def test_every_native_job_must_pass_without_duplicates(self):
        jobs = [{'name': target, 'conclusion': 'success'} for target in TARGETS]
        verify_jobs(jobs + [{'name': 'Publish release downloads', 'conclusion': 'skipped'}])
        for bad in [jobs[:-1], jobs + jobs[:1],
                    jobs[:-1] + [{'name': TARGETS[-1], 'conclusion': 'skipped'}]]:
            with self.assertRaises(ValueError):
                verify_jobs(bad)

    def test_successful_validation_needs_no_retry(self):
        release = self.fixture()
        release.validate(42, REVISION)
        self.assertEqual(release.api.call_count, 2)
        self.assertEqual(release.gh.call_args.args[:2], ('run', 'view'))

    def test_failure_cancellation_or_wrong_commit_never_reaches_job_gate(self):
        for changes in [{'conclusion': 'failure'}, {'conclusion': 'cancelled'}, {'head_sha': 'b' * 40}]:
            release = self.fixture()
            release.api.return_value = record(**changes)
            with patch('github_release.ask', return_value=''), self.assertRaises((ValueError, RuntimeError)):
                release.validate(42, REVISION)
            release.gh.assert_not_called()
            release.git.assert_not_called()

    def test_temporary_failure_can_retry_and_must_pass_new_attempt(self):
        release = self.fixture()
        failure = record(conclusion='failure')
        success = record(run_attempt=2)
        release.api.side_effect = [failure, failure, success, success, success]
        with patch('github_release.ask', return_value='y'):
            release.validate(42, REVISION)
        release.gh.assert_any_call('run', 'rerun', '42', '--failed')

    def test_retries_are_bounded(self):
        release = self.fixture()
        release.api.side_effect = [record(conclusion='failure', run_attempt=attempt)
                                  for attempt in [1, 1, 2, 2, 2, 3, 3, 3]]
        with patch('github_release.ask', return_value='y'), self.assertRaises(RuntimeError):
            release.validate(42, REVISION)
        self.assertEqual(sum(call.args[:2] == ('run', 'rerun')
                             for call in release.gh.call_args_list), 2)

    def test_confirmation_tags_the_validated_sha_and_pushes_only_that_tag(self):
        release = self.fixture()
        with patch('github_release.ask', return_value='v0.1.1'):
            release.tag(REVISION, 'v0.1.1')
        release.git.assert_any_call('tag', 'v0.1.1', REVISION)
        release.git.assert_any_call('push', '--no-follow-tags', 'origin', 'refs/tags/v0.1.1:refs/tags/v0.1.1')

    def test_cancel_existing_remote_tag_or_changed_checkout_never_pushes(self):
        for reason in ['cancel', 'remote-tag', 'changed', 'changed-after-prompt', 'wrong-local-tag']:
            release = self.fixture()
            if reason == 'remote-tag':
                release.git.return_value = REVISION + '\trefs/tags/v0.1.1'
            if reason == 'changed':
                release.clean_revision.return_value = 'b' * 40
            if reason == 'changed-after-prompt':
                release.clean_revision.side_effect = [REVISION, 'b' * 40]
            if reason == 'wrong-local-tag':
                release.run.return_value.returncode = 0
                release.git.side_effect = ['', 'b' * 40]
            with patch('github_release.ask', return_value='' if reason == 'cancel' else 'v0.1.1'):
                if reason == 'cancel':
                    release.tag(REVISION, 'v0.1.1')
                else:
                    with self.assertRaises(ValueError):
                        release.tag(REVISION, 'v0.1.1')
            self.assertFalse(any(call.args[0] in ('tag', 'push')
                                 for call in release.git.call_args_list))

    def test_dispatch_identifies_its_own_run_among_concurrent_runs(self):
        release = self.fixture()
        release.gh.side_effect = ['', json.dumps([
            {'databaseId': 41, 'displayTitle': 'Pre-release validation somebody-else'},
            {'databaseId': 42, 'displayTitle': 'Pre-release validation unique'}])]
        with patch('github_release.uuid.uuid4', return_value=Mock(hex='unique')):
            self.assertEqual(release.dispatch(REVISION, 'main'), 42)
        release.gh.assert_any_call('workflow', 'run', 'native-validation.yml', '--ref', 'main',
                                  '-f', 'request_id=unique', '-f', 'expected_revision=' + REVISION)

    def test_select_requires_pushed_commit_and_reads_committed_version(self):
        release = self.fixture()
        release.git.side_effect = ['main', 'git@github.com:owner/repo.git',
                                   'https://github.com/owner/repo.git', REVISION + '\trefs/heads/main',
                                   '[workspace.package]\nversion = "0.1.1"\n']
        self.assertEqual(release.select(), (REVISION, 'main', 'v0.1.1'))
        self.assertEqual(release.repository, 'owner/repo')
        release.git.assert_any_call('show', REVISION + ':Cargo.toml')
        release.git.side_effect = ['main', 'https://github.com/owner/repo.git',
                                   'https://github.com/owner/repo.git',
                                   'b' * 40 + '\trefs/heads/main']
        with self.assertRaisesRegex(ValueError, 'Push this branch'):
            release.select()

    def test_dirty_worktree_stops_before_selecting_or_dispatching(self):
        release = GitHubRelease()
        release.git = Mock(return_value=' M Cargo.toml')
        with self.assertRaisesRegex(ValueError, 'working-tree changes'):
            release.select()
        self.assertEqual(release.git.call_count, 1)

    def test_mismatched_push_destination_is_rejected(self):
        release = self.fixture()
        release.git.side_effect = ['main', 'git@github.com:owner/repo.git',
                                   'https://github.com/other/repo.git']
        with self.assertRaisesRegex(ValueError, 'push destination'):
            release.select()

    def test_real_git_push_tags_selected_commit_without_pushing_branch_or_extra_tags(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'source'
            root.mkdir()
            remote = Path(directory) / 'remote.git'
            def git(*arguments):
                return subprocess.run(['git', '-C', str(root), *arguments], check=True,
                                      capture_output=True, text=True).stdout.strip()
            git('init', '--quiet')
            subprocess.run(['git', 'init', '--bare', '--quiet', str(remote)], check=True,
                           capture_output=True)
            (root / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.1.1"\n')
            git('add', 'Cargo.toml')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                'commit', '--quiet', '-m', 'Fixture source')
            revision = git('rev-parse', 'HEAD')
            git('remote', 'add', 'origin', str(remote))
            git('config', 'push.followTags', 'true')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                'tag', '-a', 'unrelated', '-m', 'Do not push this tag')
            release = GitHubRelease(root=root)
            release.repository = 'owner/repo'
            with patch('github_release.ask', return_value='v0.1.1'):
                release.tag(revision, 'v0.1.1')
            self.assertEqual(git('ls-remote', 'origin'), revision + '\trefs/tags/v0.1.1')


if __name__ == '__main__':
    unittest.main()
