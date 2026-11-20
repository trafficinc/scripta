#!/usr/bin/env python3
"""Validate a pushed commit in Actions, then offer to tag it for publication."""
import argparse
import json
import re
import subprocess
import sys
import time
import uuid

from prepare_release import next_version, workspace_version
from validate_native import ROOT, TARGETS

WORKFLOW = 'native-validation.yml'
WORKFLOW_PATH = '.github/workflows/' + WORKFLOW


def repository_from_url(url):
    match = re.fullmatch(r'(?:https://github\.com/|ssh://git@github\.com/|git@github\.com:)'
                         r'([\w.-]+/[\w.-]+?)(?:\.git)?/?', url)
    if not match:
        raise ValueError('Selected remote must point to a github.com repository')
    return match[1]


def ask(prompt):
    try:
        return input(prompt).strip()
    except EOFError:
        return ''


def verify_run(record, revision):
    if (record.get('head_sha') != revision or record.get('event') != 'workflow_dispatch'
            or record.get('path') != WORKFLOW_PATH):
        raise ValueError('Actions run does not validate this commit with the expected manual workflow')


def verify_jobs(jobs):
    native = [job for job in jobs if job['name'] in TARGETS]
    if (len(native) != len(TARGETS) or {job['name'] for job in native} != set(TARGETS)
            or any(job.get('conclusion') != 'success' for job in native)):
        raise ValueError('All four native validation jobs must succeed before tagging')


class GitHubRelease:
    def __init__(self, remote='origin', root=ROOT):
        self.root = root
        self.remote = remote
        self.repository = None

    def run(self, arguments, check=True, stream=False):
        print('+ ' + ' '.join(map(str, arguments)), flush=True)
        result = subprocess.run(arguments, cwd=self.root, text=True, capture_output=not stream)
        if check and result.returncode:
            raise RuntimeError((result.stderr or '').strip() or (result.stdout or '').strip() or
                               'Command failed: ' + ' '.join(arguments))
        return result

    def git(self, *arguments):
        return self.run(['git', *arguments]).stdout.strip()

    def gh(self, *arguments):
        return self.run(['gh', *arguments, '--repo', self.repository]).stdout

    def api(self, suffix):
        return json.loads(self.run(['gh', 'api', f'repos/{self.repository}/{suffix}']).stdout)

    def clean_revision(self):
        if self.git('status', '--porcelain'):
            raise ValueError('Commit or set aside working-tree changes before running this script')
        return self.git('rev-parse', 'HEAD')

    def select(self):
        revision = self.clean_revision()
        branch = self.git('symbolic-ref', '--quiet', '--short', 'HEAD')
        origin = self.git('remote', 'get-url', self.remote)
        self.repository = repository_from_url(origin)
        push_urls = self.git('remote', 'get-url', '--push', '--all', self.remote).splitlines()
        if (len(push_urls) != 1 or
                repository_from_url(push_urls[0]).lower() != self.repository.lower()):
            raise ValueError('Remote must have one push destination matching the validated repository')
        remote_head = self.git('ls-remote', '--exit-code', self.remote, 'refs/heads/' + branch)
        if remote_head.split()[0] != revision:
            raise ValueError('Push this branch so its remote HEAD matches the selected commit')
        manifest = self.git('show', revision + ':Cargo.toml')
        version = workspace_version(manifest)
        next_version(version, 'patch')  # Require a stable version before dispatching.
        return revision, branch, 'v' + version

    def dispatch(self, revision, branch):
        request = uuid.uuid4().hex
        title = 'Pre-release validation ' + request
        self.gh('workflow', 'run', WORKFLOW, '--ref', branch,
                '-f', 'request_id=' + request, '-f', 'expected_revision=' + revision)
        print('Finding the validation run...', flush=True)
        for _ in range(60):
            runs = json.loads(self.gh('run', 'list', '--workflow', WORKFLOW,
                                     '--event', 'workflow_dispatch', '--limit', '100',
                                     '--json', 'databaseId,displayTitle'))
            matching = [run for run in runs if run['displayTitle'] == title]
            if len(matching) == 1:
                return matching[0]['databaseId']
            if len(matching) > 1:
                raise ValueError('Multiple Actions runs matched the dispatch request')
            time.sleep(2)
        raise RuntimeError('Run discovery timed out. Find the run in Actions and resume with --run-id ID')

    def validate(self, run_id, revision):
        # Read the run from this exact repository, including when resuming.
        for attempt in range(3):
            record = self.api(f'actions/runs/{run_id}')
            verify_run(record, revision)
            print('Validation: ' + record['html_url'], flush=True)
            if record['status'] != 'completed':
                self.run(['gh', 'run', 'watch', str(run_id), '--repo', self.repository,
                          '--exit-status'], check=False, stream=True)
            record = self.api(f'actions/runs/{run_id}')
            verify_run(record, revision)
            if record['status'] != 'completed':
                raise RuntimeError(f'Validation is still running; resume with --run-id {run_id}')
            if record.get('conclusion') == 'success':
                view = json.loads(self.gh('run', 'view', str(run_id), '--json', 'jobs'))
                verify_jobs(view['jobs'])
                return
            print(f'Validation ended with {record.get("conclusion")}. Inspect the Actions logs.')
            if attempt == 2 or ask('Retry failed jobs for a temporary failure? [y/N]: ').lower() != 'y':
                raise RuntimeError('Validation did not pass. Fix code failures in a new commit before retrying')
            self.gh('run', 'rerun', str(run_id), '--failed')
            # Wait for GitHub to register the rerun before checking its results.
            previous = record['run_attempt']
            for _ in range(60):
                if self.api(f'actions/runs/{run_id}')['run_attempt'] > previous:
                    break
                time.sleep(2)
            else:
                raise RuntimeError(f'Rerun has not started; resume with --run-id {run_id}')

    def tag(self, revision, tag):
        if self.clean_revision() != revision:
            raise ValueError('Checkout changed during validation; rerun for the intended commit')
        existing = self.git('ls-remote', self.remote, 'refs/tags/' + tag)
        if existing:
            raise ValueError(f'Remote tag {tag} already exists; existing tags are never replaced')
        local = self.run(['git', 'show-ref', '--verify', '--quiet', 'refs/tags/' + tag], check=False)
        if local.returncode == 0:
            if self.git('rev-parse', 'refs/tags/' + tag + '^{commit}') != revision:
                raise ValueError(f'Local tag {tag} points to another commit')
        elif local.returncode != 1:
            raise RuntimeError('Could not inspect local tags')
        print(f'All four platforms passed for {revision}.')
        print(f'Tagging {self.repository} will trigger a fresh build and GitHub Release publication.')
        if ask(f'Type {tag} to create/push that release tag, or Enter to cancel: ') != tag:
            print('Cancelled. No tag was created or pushed.')
            return
        if self.clean_revision() != revision:
            raise ValueError('Checkout changed while awaiting confirmation')
        if local.returncode == 1:
            self.git('tag', tag, revision)
        # Push the explicit tag only; never push the branch or replace a remote tag.
        self.git('push', '--no-follow-tags', self.remote, f'refs/tags/{tag}:refs/tags/{tag}')
        print(f'Tag {tag} pushed for {revision}.')
        print(f'Release workflow: https://github.com/{self.repository}/actions/workflows/{WORKFLOW}')
        print(f'Release will appear after all tag-workflow checks pass: '
              f'https://github.com/{self.repository}/releases/tag/{tag}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--remote', default='origin', help='GitHub remote to validate and tag (default: origin)')
    parser.add_argument('--run-id', type=int, help='resume an existing manual validation run for this commit')
    args = parser.parse_args()
    try:
        release = GitHubRelease(args.remote)
        revision, branch, tag = release.select()
        run_id = args.run_id or release.dispatch(revision, branch)
        print(f'Resume this validation with: python3 scripts/github_release.py '
              f'--remote {args.remote} --run-id {run_id}', flush=True)
        release.validate(run_id, revision)
        release.tag(revision, tag)
    except KeyboardInterrupt:
        print('\nStopped. Any dispatched Actions run continues; use --run-id to resume.', file=sys.stderr)
        return 130
    except (OSError, ValueError, RuntimeError) as error:
        print('GitHub release failed: ' + str(error), file=sys.stderr)
        print('If a tag was created before a push failed, it remains local for inspection or retry.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
