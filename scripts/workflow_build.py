#!/usr/bin/env python3
"""Validate candidate versions and assemble read-only release build artifacts."""
import argparse
import json
import os
from pathlib import Path
import re

from prepare_release import workspace_version
from promote_release import GitHub, checked_evidence, package_names, tag_path
from publish_release import prepare
from validate_native import ROOT
from release_assets import public_assets, validation_directory


def build_options(environment, root=ROOT):
    revision = environment.get('GITHUB_SHA', '')
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        raise ValueError('A full GitHub source commit SHA is required')
    version = workspace_version((root / 'Cargo.toml').read_text(encoding='utf-8'))
    if not re.fullmatch(r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)', version):
        raise ValueError('Builds require a committed stable workspace version')
    if (environment.get('GITHUB_EVENT_NAME') != 'workflow_dispatch'
            or environment.get('GITHUB_REF') != 'refs/heads/main'):
        raise ValueError('Run Build release candidate manually on main after committing and pushing the prepared version')
    return {'version': version, 'tag': 'v' + version, 'source_revision': revision,
            'prerelease': False}


def check_available(api, options):
    tag = options['tag']
    if (api.get('/releases/tags/' + tag, missing=True) is not None
            or api.get(tag_path(tag), missing=True) is not None):
        raise ValueError(f'{tag} already has a release or tag. Run scripts/prepare_release.py, '
                         'choose a new version, commit and push it, then build main again')


def assemble(inputs, output, options):
    return prepare(inputs, output, options['source_revision'], options['tag'], options['version'])


def verify_saved_assets(assets, options, scratch):
    public_assets(assets, options['version'])
    inputs = checked_evidence(assets, options['tag'], options['source_revision'],
                              options['version'], scratch, prerelease=options['prerelease'])
    # Reapply the publication gates to the bundled native records, not just hashes.
    manifest, notes = assemble(inputs, scratch / 'verified', options)
    original = json.loads((scratch / 'evidence/release-manifest.json').read_bytes())
    if original != manifest:
        raise ValueError('Saved build manifest differs from verified native evidence')
    return manifest, notes


def write_summary(options, environment):
    text = (f"## Scripta {options['version']} build is ready\n\n"
            f"Source commit: `{options['source_revision']}`\n\n"
            f"Release tag: `{options['tag']}`\n\n"
            f"Build run ID: **{environment['GITHUB_RUN_ID']}**\n\n"
            'All three native test/package jobs passed: Linux x86-64, '
            'Windows x86-64, and macOS 15 Apple Silicon. SQLite is included.\n\n'
            f"[Download the validated build]({environment['BUILD_ARTIFACT_URL']})\n\n"
            'The internal build contains an `assets` directory with three platform archives '
            'and `SHA256SUMS`, plus separate local validation records. '
            'Only the contents of `assets` are published. Artifacts expire after 30 days.\n\n')
    text += ('Next: open **Publish release candidate**, paste this build run ID, '
             'and run it on the default branch. The release tag is derived automatically.\n')
    with Path(environment['GITHUB_STEP_SUMMARY']).open('a', encoding='utf-8', newline='\n') as output:
        output.write(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('inspect', 'prepare', 'summary'))
    parser.add_argument('--inputs', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    try:
        options = build_options(os.environ)
        if args.command == 'inspect':
            api = GitHub(os.environ['GITHUB_REPOSITORY'], os.environ.get('GH_TOKEN'))
            check_available(api, options)
            print(json.dumps(options))
        elif args.command == 'prepare':
            if args.inputs is None or args.output is None:
                parser.error('prepare requires --inputs and --output')
            assemble(args.inputs, args.output, options)
        elif args.command == 'summary':
            write_summary(options, os.environ)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, 'Build workflow failed: ' + str(error) + '\n')


if __name__ == '__main__':
    main()
