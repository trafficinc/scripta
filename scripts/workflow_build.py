#!/usr/bin/env python3
"""Assemble manual/tag builds and optionally publish their saved, verified assets."""
import argparse
import html
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import urllib.parse

from prepare_release import workspace_version
from promote_release import GitHub, checked_evidence, package_names, resolve_tag, tag_path
from publish_release import prepare, publish, release_kind
from validate_native import ROOT


def build_options(environment, root=ROOT):
    revision = environment.get('GITHUB_SHA', '')
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        raise ValueError('A full GitHub source commit SHA is required')
    expected = environment.get('BUILD_EXPECTED_REVISION', '')
    if expected and expected != revision:
        raise ValueError('Selected revision differs from the expected revision')
    version = workspace_version((root / 'Cargo.toml').read_text(encoding='utf-8'))
    if not re.fullmatch(r'(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)', version):
        raise ValueError('Builds require a committed stable workspace version')
    event = environment.get('GITHUB_EVENT_NAME')
    if event == 'push' and environment.get('GITHUB_REF_TYPE') == 'tag':
        tag = environment['GITHUB_REF_NAME']
        create_release = True
    elif event == 'workflow_dispatch':
        choice = environment.get('BUILD_CREATE_RELEASE', 'false')
        if choice not in ('true', 'false'):
            raise ValueError('Create release must be true or false')
        create_release = choice == 'true'
        tag = environment.get('BUILD_RELEASE_TAG') or 'v' + version
    else:
        raise ValueError('Builds require manual dispatch or a tag push')
    prerelease = release_kind(tag, version)
    subprocess.run(['git', 'check-ref-format', 'refs/tags/' + tag], check=True,
                   capture_output=True, text=True)
    return {'version': version, 'tag': tag, 'source_revision': revision,
            'prerelease': prerelease, 'create_release': create_release}


def assemble(inputs, output, options):
    return prepare(inputs, output, options['source_revision'], options['tag'], options['version'])


def verify_saved_assets(assets, options, scratch):
    expected = set(package_names(options['version']).values()) | {'SHA256SUMS', 'validation-evidence.zip'}
    if ({path.name for path in assets.iterdir()} != expected
            or any(not path.is_file() or path.is_symlink() for path in assets.iterdir())):
        raise ValueError('Saved build must contain exactly the six release assets')
    inputs = checked_evidence(assets, options['tag'], options['source_revision'],
                              options['version'], scratch, prerelease=options['prerelease'])
    # Reapply the publication gates to the bundled native records, not just hashes.
    manifest, notes = assemble(inputs, scratch / 'verified', options)
    original = json.loads((scratch / 'evidence/release-manifest.json').read_bytes())
    if original != manifest:
        raise ValueError('Saved build manifest differs from verified native evidence')
    return manifest, notes


def publish_saved(assets, options, environment, api, publisher=publish):
    if (not options['create_release'] or environment.get('GITHUB_ACTIONS') != 'true'
            or not environment.get('GH_TOKEN')):
        raise ValueError('Publication requires an authorized Actions release run')
    repository = environment['GITHUB_REPOSITORY']
    with tempfile.TemporaryDirectory(prefix='scripta-saved-build-') as directory:
        manifest, notes = verify_saved_assets(assets, options, Path(directory))
        tag = manifest['tag']
        revision = manifest['source_revision']
        existing = api.get('/releases/tags/' + urllib.parse.quote(tag, safe=''), missing=True)
        if existing is not None and not existing.get('draft'):
            raise ValueError('This release is already published; it will not be replaced')
        reference = api.get(tag_path(tag), missing=True)
        if reference is None:
            if environment.get('GITHUB_EVENT_NAME') != 'workflow_dispatch':
                raise ValueError('The pushed release tag no longer exists')
            # The built-in workflow token creates this tag without another build.
            api.post('/git/refs', {'ref': 'refs/tags/' + tag, 'sha': revision})
        if resolve_tag(api, tag) != revision:
            raise ValueError('Release tag differs from the validated commit; it will not be moved')
        if environment.get('GITHUB_EVENT_NAME') == 'workflow_dispatch':
            extra = environment.get('BUILD_RELEASE_NOTES', '')
            if extra:
                text = '\n'.join(extra.splitlines()) + '\n\n' + notes.read_text(encoding='utf-8')
                notes.write_bytes(text.encode('utf-8'))
        # Publish the original downloaded files. Revalidation never replaces them.
        return publisher(repository, manifest, assets, notes)


def write_summary(options, environment):
    text = (f"## Scripta {options['version']} build is ready\n\n"
            f"Source commit: `{options['source_revision']}`\n\n"
            f"Package tag: <code>{html.escape(options['tag'])}</code>\n\n"
            'All four native test/package jobs passed: Linux x86-64/ARM64, '
            'Windows x86-64, and macOS 15 Apple Silicon. SQLite is included.\n\n'
            f"[Download the validated build]({environment['BUILD_ARTIFACT_URL']})\n\n"
            'The download contains four platform archives, `SHA256SUMS`, and '
            '`validation-evidence.zip`. Artifacts expire after 30 days.\n\n')
    text += ('Release publication follows in **Publish release downloads**.\n'
             if options['create_release'] else 'Build only; no tag or GitHub Release was created.\n')
    with Path(environment['GITHUB_STEP_SUMMARY']).open('a', encoding='utf-8', newline='\n') as output:
        output.write(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('inspect', 'prepare', 'summary', 'publish'))
    parser.add_argument('--inputs', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--assets', type=Path)
    args = parser.parse_args()
    try:
        options = build_options(os.environ)
        if args.command == 'inspect':
            print(json.dumps(options))
        elif args.command == 'prepare':
            if args.inputs is None or args.output is None:
                parser.error('prepare requires --inputs and --output')
            assemble(args.inputs, args.output, options)
        elif args.command == 'summary':
            write_summary(options, os.environ)
        else:
            if args.assets is None:
                parser.error('publish requires --assets')
            api = GitHub(os.environ['GITHUB_REPOSITORY'], os.environ.get('GH_TOKEN'))
            result = publish_saved(args.assets, options, os.environ, api)
            print(result)
            url = json.loads(result)['url']
            with Path(os.environ['GITHUB_STEP_SUMMARY']).open('a', encoding='utf-8', newline='\n') as output:
                output.write(f'## Release published\n\n[Open GitHub Release]({url})\n')
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, 'Build workflow failed: ' + str(error) + '\n')


if __name__ == '__main__':
    main()
