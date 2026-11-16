#!/usr/bin/env python3
"""Run local checks, then interactively bump the version for a GitHub tag release."""
import argparse
import json
import os
import platform
import re
import subprocess
import sys

from validate_native import ROOT, TARGETS


def next_version(version, choice):
    if not re.fullmatch(r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)', version):
        raise ValueError('Expected a stable major.minor.patch workspace version')
    major, minor, patch = map(int, version.split('.'))
    return {'patch': f'{major}.{minor}.{patch + 1}',
            'minor': f'{major}.{minor + 1}.0',
            'major': f'{major + 1}.0.0'}[choice]


def choose_version(version):
    choices = {choice: next_version(version, choice) for choice in ('patch', 'minor', 'major')}
    print('\nAll local checks passed. Choose the version bump:')
    for choice, candidate in choices.items():
        print(f'  {choice}: {version} -> {candidate}')
    while True:
        try:
            choice = input('patch / minor / major / cancel [cancel]: ').strip().lower()
        except EOFError:
            return None
        if choice in ('', 'cancel'):
            return None
        if choice in choices:
            return choices[choice]
        print('Enter patch, minor, major, or cancel.')


class Preparation:
    def __init__(self, root=ROOT, offline=False):
        self.root = root
        self.offline = offline
        self.environment = os.environ.copy()
        channel = re.search(r'^channel\s*=\s*"([^"]+)"',
                            (root / 'rust-toolchain.toml').read_text(encoding='utf-8'), re.M)
        if not channel:
            raise ValueError('Missing pinned Rust channel')
        self.environment['RUSTUP_TOOLCHAIN'] = channel[1]

    def run(self, arguments, capture=False, check=True):
        print('+ ' + ' '.join(map(str, arguments)), flush=True)
        return subprocess.run([str(item) for item in arguments], cwd=self.root,
                              env=self.environment, check=check, text=True,
                              stdout=subprocess.PIPE if capture else None)

    def cargo(self, *arguments, capture=False):
        return self.run(['cargo', *arguments, *(['--offline'] if self.offline else [])],
                        capture=capture)

    def checks(self):
        compiler = self.run(['rustc', '-vV'], capture=True).stdout
        print(compiler, end='')
        host = re.search(r'^host: (\S+)$', compiler, re.M)
        if not host or host[1] not in TARGETS:
            raise ValueError('Compiler host must be one of the four native release targets')
        target = host[1]
        if target.endswith('apple-darwin'):
            self.environment['MACOSX_DEPLOYMENT_TARGET'] = '15.0'
        print('Native OS: ' + platform.platform(), flush=True)
        self.run(['git', 'rev-parse', 'HEAD'])
        self.run(['cargo', 'fmt', '--all', '--', '--check'])
        self.run([sys.executable, '-m', 'unittest', 'discover', '-s', 'scripts', '-p', 'test_*.py'])
        for arguments in [
            ['test', '-p', 'scr-stdlib', '--lib', 'collections'],
            ['test', '-p', 'scr-stdlib', '--test', 'collections'],
            ['test', '--workspace'],
            ['test', '--workspace', '--all-features'],
            ['test', '-p', 'scr-cli', '--no-default-features', '--test', 'sqlite_extension'],
        ]:
            self.cargo(*arguments, '--locked', '--target', target)
        # Cargo options must precede the Clippy argument separator.
        self.run(['cargo', 'clippy', '--workspace', '--all-targets', '--all-features',
                  '--locked', '--target', target, *(['--offline'] if self.offline else []),
                  '--', '-D', 'warnings'])
        self.run(['git', 'diff', '--check'])

    def bump(self, snapshots, version, candidate):
        if any(path.read_bytes() != data for path, data in snapshots.items()):
            raise ValueError('Version files changed during checks; rerun before bumping')
        tag = 'v' + candidate
        exists = self.run(['git', 'show-ref', '--verify', '--quiet', 'refs/tags/' + tag], check=False)
        if exists.returncode == 0:
            raise ValueError(f'Tag {tag} already exists; choose a new version')
        if exists.returncode != 1:
            raise RuntimeError('Could not check existing local tags')
        metadata = json.loads(self.cargo('metadata', '--format-version', '1', '--no-deps',
                                         '--locked', capture=True).stdout)
        members = set(metadata['workspace_members'])
        names = {package['name'] for package in metadata['packages'] if package['id'] in members}
        manifest = self.root / 'Cargo.toml'
        lock = self.root / 'Cargo.lock'
        updated_manifest = replace_workspace_version(snapshots[manifest].decode('utf-8'), candidate)
        # Change only local workspace package versions, preserving dependency pins.
        blocks = re.split(r'(?=^\[\[package\]\]\r?$)', snapshots[lock].decode('utf-8'), flags=re.M)
        found = set()
        for index, block in enumerate(blocks):
            name = re.search(r'^name = "([^"]+)"\r?$', block, re.M)
            if not name or name[1] not in names or re.search(r'^source\s*=', block, re.M):
                continue
            old = re.search(r'^version = "([^"]+)"\r?$', block, re.M)
            if not old or old[1] != version or name[1] in found:
                raise ValueError('Unexpected workspace package version in Cargo.lock')
            found.add(name[1])
            blocks[index] = block[:old.start(1)] + candidate + block[old.end(1):]
        if found != names or not names:
            raise ValueError('Cargo.lock does not contain every workspace package')
        try:
            manifest.write_bytes(updated_manifest.encode('utf-8'))
            lock.write_bytes(''.join(blocks).encode('utf-8'))
            # A full locked resolve checks that Cargo accepts the updated lockfile.
            self.cargo('metadata', '--format-version', '1', '--locked', capture=True)
            self.run(['git', 'diff', '--check'])
        except BaseException:
            for path, data in snapshots.items():
                path.write_bytes(data)
            raise
        return tag


def workspace_version(text):
    section = re.search(r'^\[workspace\.package\]\s*\n(.*?)(?=^\[|\Z)', text, re.M | re.S)
    match = re.search(r'^version\s*=\s*"([^"]+)"', section[1], re.M) if section else None
    if not match:
        raise ValueError('Missing workspace package version')
    return match[1]


def replace_workspace_version(text, candidate):
    version = workspace_version(text)
    section = re.search(r'^\[workspace\.package\]\s*\n(.*?)(?=^\[|\Z)', text, re.M | re.S)
    match = re.search(r'^version\s*=\s*"(' + re.escape(version) + r')"', section[1], re.M)
    start = section.start(1) + match.start(1)
    return text[:start] + candidate + text[start + len(version):]


def prepare(preparation):
    snapshots = {preparation.root / name: (preparation.root / name).read_bytes()
                 for name in ('Cargo.toml', 'Cargo.lock')}
    version = workspace_version(snapshots[preparation.root / 'Cargo.toml'].decode('utf-8'))
    next_version(version, 'patch')  # Reject unsupported versions before expensive checks.
    preparation.checks()
    candidate = choose_version(version)
    if candidate is None:
        print('Cancelled. Version files unchanged.')
        return
    tag = preparation.bump(snapshots, version, candidate)
    print(f'\nUpdated Cargo.toml and Cargo.lock: {version} -> {candidate}.')
    print('Review the changes and release notes, then commit the intended source revision.')
    print('After committing and pushing the branch, validate all four platforms before tagging:')
    print('  python3 scripts/github_release.py')
    print('It waits for GitHub validation, then asks you to confirm the matching release tag.')
    print('Alternatively, to trigger release validation directly with a tag:')
    print(f'  git tag {tag}')
    print(f'  git push origin refs/tags/{tag}')
    print('The tag workflow builds all four native packages and publishes to GitHub Releases')
    print('only after every native validation job passes. Other-platform validation is pending.')
    print('No commit, tag, push, local package, or GitHub release was created by this script.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--offline', action='store_true', help='use cached Cargo dependencies only')
    arguments = parser.parse_args()
    try:
        prepare(Preparation(offline=arguments.offline))
    except KeyboardInterrupt:
        print('\nCancelled.', file=sys.stderr)
        return 130
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print('Release preparation failed: ' + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
