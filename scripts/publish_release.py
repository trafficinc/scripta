#!/usr/bin/env python3
"""Prepare verified native assets, then publish a tag release using GitHub CLI."""
import argparse
import json
import os
import re
import shutil
import subprocess
import tempfile
import zipfile
from pathlib import Path

from validate_native import ROOT, TARGETS, digest

SUITES = {'tests-' + profile + '-' + features
          for profile in ('debug', 'dist') for features in ('default', 'all-features', 'minimal')}


def release_kind(tag, version):
    if not tag or tag.startswith('-') or any(ord(c) < 32 for c in tag):
        raise ValueError('Invalid release tag')
    version_tag = re.fullmatch(r'v?(\d+\.\d+\.\d+)(?:[-+].+)?', tag)
    if version_tag and version_tag[1] != version.split('-')[0].split('+')[0]:
        raise ValueError('Version tag does not match the workspace version')
    return tag != 'v' + version or '-' in version


def prepare(inputs, output, revision, tag, version):
    prerelease = release_kind(tag, version)
    if output.exists():
        raise ValueError('Choose a fresh release asset directory')
    records = {}
    for path in inputs.glob('*/validation.json'):
        record = json.loads(path.read_text(encoding='utf-8'))
        target = record['target']
        if target not in TARGETS or target in records:
            raise ValueError('Unexpected or duplicate native target')
        if record.get('status') != 'passed' or record.get('source_revision') != revision:
            raise ValueError('Native validation did not pass for the tagged revision')
        if record.get('version') != version or set(record.get('tests', {})) != SUITES:
            raise ValueError('Native version or suite coverage differs')
        for suite in record['tests'].values():
            if suite.get('passed', 0) <= 0 or suite.get('failed') != 0 or suite.get('ignored') != 0:
                raise ValueError('Native tests failed, were ignored, or are missing')
        if not record.get('steps') or any(s.get('exit_code') != 0 for s in record['steps']):
            raise ValueError('Native validation contains unsuccessful steps')
        if set(record.get('packages', {})) != {'standard'}:
            raise ValueError('One standard SQLite-enabled package is required per target')
        records[target] = (path, record)
    if set(records) != set(TARGETS):
        raise ValueError('All four native targets must pass before publication')

    # Validate every input before staging or contacting GitHub.
    packages = []
    for target, (path, record) in sorted(records.items()):
        root = path.parent.resolve()
        extension = '.zip' if target.endswith('windows-msvc') else '.tar.gz'
        for variant, package in sorted(record['packages'].items()):
            if package.get('extensions') != ['sqlite']:
                raise ValueError('Standard packages must include SQLite')
            expected = f'scripta-{version}-{target}{extension}'
            relative = Path(package['archive'].replace('\\', '/'))
            if relative.as_posix() != f'{variant}/{expected}':
                raise ValueError('Unexpected native archive path')
            archive = (root / relative).resolve()
            archive.relative_to(root)
            if digest(archive) != package['archive_sha256']:
                raise ValueError('Native archive checksum mismatch')
            if (package.get('offline_package_smoke') != 'passed'
                    or package.get('identical_input_repack') != 'passed'
                    or not package.get('dependencies')):
                raise ValueError('Native package evidence is incomplete')
            smoke = archive.with_name(archive.name + '.validation.json')
            evidence = json.loads(smoke.read_text(encoding='utf-8'))
            for key, value in [('target', target), ('version', version), ('extensions', ['sqlite']),
                               ('archive_sha256', package['archive_sha256']),
                               ('binary_sha256', package['binary_sha256'])]:
                if evidence.get(key) != value:
                    raise ValueError('Extracted-package evidence differs from archive')
            name = expected
            packages.append((archive, smoke, name, target, variant))

    output.mkdir(parents=True)
    manifest = {'tag': tag, 'source_revision': revision, 'version': version,
                'prerelease': prerelease, 'packages': []}
    with tempfile.TemporaryDirectory(prefix='scripta-evidence-', dir=output.parent) as directory:
        evidence = Path(directory)
        for archive, smoke, name, target, variant in packages:
            shutil.copyfile(archive, output / name)
            checksum = digest(output / name)
            (evidence / (name + '.sha256')).write_text(f'{checksum}  {name}\n', encoding='utf-8')
            shutil.copyfile(smoke, evidence / (name + '.validation.json'))
            manifest['packages'].append({'asset': name, 'target': target, 'variant': variant,
                                         'archive_sha256': checksum, 'extensions': ['sqlite']})
        for target, (path, _) in sorted(records.items()):
            shutil.copyfile(path, evidence / f'validation-{target}.json')
            logs = evidence / 'logs' / target
            logs.mkdir(parents=True)
            for log in sorted((path.parent / 'logs').glob('*.log')):
                shutil.copyfile(log, logs / log.name)
        manifest['files'] = {p.name: digest(p) for p in sorted(output.iterdir())}
        manifest['files'].update({p.relative_to(evidence).as_posix(): digest(p)
                                  for p in sorted(evidence.rglob('*')) if p.is_file()})
        (evidence / 'release-manifest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
        with zipfile.ZipFile(output / 'validation-evidence.zip', 'w', zipfile.ZIP_DEFLATED) as bundle:
            for path in sorted(evidence.rglob('*')):
                if path.is_file():
                    bundle.write(path, path.relative_to(evidence).as_posix())
    checksums = [f'{digest(path)}  {path.name}\n' for path in sorted(output.iterdir())]
    (output / 'SHA256SUMS').write_text(''.join(checksums), encoding='utf-8')
    notes = [f'Scripta {version}', '', f'Tag: `{tag}`', f'Source: `{revision}`', '',
             'Native validation passed on Linux x86-64/ARM64, Windows x86-64, and macOS 15 Apple Silicon.',
             'Default/all-feature debug/dist suites, minimal-build compatibility tests, Clippy, dependency inspection, archive verification,',
             'offline extracted-package smoke, and identical-input repacking passed.', '',
             'Download the single archive for your platform; SQLite is included by default.',
             'Archives contain `scripta` and legacy `scr` commands; no Rust installation is required.',
             'SHA256SUMS verifies the four packages and validation-evidence.zip.',
             'The evidence ZIP contains individual checksums, native/smoke records, logs, and release-manifest.json.',
             'Candidates have no publisher signatures or macOS notarization.']
    notes_path = output.parent / (output.name + '-notes.md')
    notes_path.write_text('\n'.join(notes) + '\n', encoding='utf-8')
    return manifest, notes_path


def publish(repository, manifest, output, notes, run=subprocess.run):
    tag = manifest['tag']
    def gh(*arguments, check=True):
        return run(['gh', 'release', *arguments, '--repo', repository],
                   check=check, capture_output=True, text=True)
    existing = gh('view', tag, '--json', 'isDraft,url', check=False)
    if existing.returncode == 0 and not json.loads(existing.stdout)['isDraft']:
        raise ValueError('This tag already has a published release; use a new tag instead of replacing it')
    flags = ['--title', 'Scripta ' + tag, '--notes-file', str(notes)]
    if existing.returncode:
        gh('create', tag, '--verify-tag', '--target', manifest['source_revision'], '--draft',
           *flags, *(['--prerelease'] if manifest['prerelease'] else []))
    else:
        gh('edit', tag, *flags)
    gh('upload', tag, *[str(path) for path in sorted(output.iterdir())], '--clobber')
    gh('edit', tag, '--draft=false', '--prerelease=' + str(manifest['prerelease']).lower(),
       '--latest=' + str(not manifest['prerelease']).lower())
    return gh('view', tag, '--json', 'url').stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    if os.environ.get('GITHUB_REF_TYPE') != 'tag':
        parser.error('Release publishing requires a tag push')
    workspace = (ROOT / 'Cargo.toml').read_text(encoding='utf-8').split('[workspace.package]', 1)[1]
    version = re.search(r'^version\s*=\s*"([^"]+)"', workspace, re.M)[1]
    manifest, notes = prepare(args.inputs, args.output, os.environ['GITHUB_SHA'],
                              os.environ['GITHUB_REF_NAME'], version)
    if not args.prepare_only:
        try:
            print(publish(os.environ['GITHUB_REPOSITORY'], manifest, args.output, notes))
        except subprocess.CalledProcessError as error:
            raise SystemExit(error.stderr or str(error)) from error


if __name__ == '__main__':
    main()
