#!/usr/bin/env python3
"""Publish packages from a successful native Actions build without compiling."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import urllib.parse
import zipfile

from promote_release import GitHub, JSON_LIMIT, SHA, WORKFLOW, require_dispatch, tag_path, workspace_version
from publish_release import prepare, publish, release_kind
from validate_native import TARGETS, digest
from workflow_build import verify_saved_assets


def successful_run(api, repository, run_id):
    if type(run_id) is not int or run_id <= 0:
        raise ValueError('Build run ID must be a positive integer')
    run = api.get('/actions/runs/' + str(run_id))
    if (run.get('id') != run_id or run.get('path') != WORKFLOW
            or run.get('event') not in ('push', 'workflow_dispatch')
            or run.get('status') != 'completed' or run.get('conclusion') != 'success'
            or not SHA.fullmatch(run.get('head_sha', ''))
            or type(run.get('run_attempt')) is not int or run['run_attempt'] <= 0
            or any(run.get(key, {}).get('full_name', '').lower() != repository.lower()
                   for key in ('repository', 'head_repository'))):
        raise ValueError('Select a successful native build from this repository')
    jobs = api.get(f'/actions/runs/{run_id}/jobs?per_page=100')
    expected = set(TARGETS) | {'Assemble validated downloads', 'Publish release downloads'}
    items = jobs.get('jobs', [])
    if (jobs.get('total_count') != len(expected) or len(items) != len(expected)
            or {job.get('name') for job in items} != expected
            or any(job.get('status') != 'completed'
                   or job.get('conclusion') not in (('success', 'skipped')
                       if job.get('name') == 'Publish release downloads' else ('success',))
                   for job in items)):
        raise ValueError('All four native jobs and artifact assembly must have passed')
    return run


def build_artifact(api, run):
    found = []
    for page in range(1, 101):
        response = api.get(f'/actions/runs/{run["id"]}/artifacts?per_page=100&page={page}')
        for artifact in response.get('artifacts', []):
            match = re.fullmatch(r'scripta-build-' + str(run['id']) + r'-([1-9][0-9]*)',
                                 artifact.get('name', ''))
            if match and int(match[1]) <= run['run_attempt']:
                found.append((int(match[1]), artifact))
        if len(response.get('artifacts', [])) < 100:
            break
    else:
        raise ValueError('Build has too many artifacts to inspect')
    if not found:
        raise ValueError('This run has no saved build; run Native builds and releases first')
    latest = max(attempt for attempt, _ in found)
    matches = [artifact for attempt, artifact in found if attempt == latest]
    if len(matches) != 1 or matches[0].get('expired') is not False:
        raise ValueError('Saved build artifact is expired or ambiguous; create a new build')
    artifact = matches[0]
    if (type(artifact.get('id')) is not int or artifact['id'] <= 0
            or artifact.get('workflow_run', {}).get('head_sha', run['head_sha']) != run['head_sha']):
        raise ValueError('Saved artifact identity differs from the build')
    return artifact


def select_build(api, repository, tag, run_id=None):
    if run_id is None:
        # GitHub returns workflow runs newest first. Explicit IDs may select other branches.
        response = api.get('/actions/workflows/native-validation.yml/runs?branch=main&status=success&per_page=1')
        runs = response.get('workflow_runs', [])
        if not runs or runs[0].get('head_branch') != 'main':
            raise ValueError('No successful native build exists on main')
        run_id = runs[0]['id']
    run = successful_run(api, repository, run_id)
    version = workspace_version(api, run['head_sha'])
    release_kind(tag, version)
    subprocess.run(['git', 'check-ref-format', 'refs/tags/' + tag],
                   check=True, capture_output=True, text=True)
    artifact = build_artifact(api, run)
    return {'repository': repository, 'run_id': run_id, 'run_attempt': run['run_attempt'],
            'source_revision': run['head_sha'], 'version': version, 'release_tag': tag,
            'artifact_id': artifact['id'], 'artifact_name': artifact['name']}


def recheck(api, selection):
    run = successful_run(api, selection['repository'], selection['run_id'])
    if (run['head_sha'] != selection['source_revision']
            or run['run_attempt'] != selection['run_attempt']):
        raise ValueError('Selected build changed or was retried during publication')
    artifact = build_artifact(api, run)
    if (artifact['id'], artifact['name']) != (selection['artifact_id'], selection['artifact_name']):
        raise ValueError('Selected build artifact changed during publication')


def prepare_saved(api, selection, assets, output):
    recheck(api, selection)
    # Read only the bounded manifest here; verify_saved_assets validates the entire ZIP.
    with zipfile.ZipFile(assets / 'validation-evidence.zip') as bundle:
        info = bundle.getinfo('release-manifest.json')
        if info.file_size > JSON_LIMIT:
            raise ValueError('Build manifest exceeds its size limit')
        with bundle.open(info) as stream:
            content = stream.read(JSON_LIMIT + 1)
        if len(content) > JSON_LIMIT:
            raise ValueError('Build manifest exceeds its size limit')
        original = json.loads(content)
    source_options = {'version': selection['version'], 'tag': original['tag'],
                      'source_revision': selection['source_revision'],
                      'prerelease': release_kind(original['tag'], selection['version'])}
    with tempfile.TemporaryDirectory(prefix='scripta-publish-build-') as directory:
        scratch = Path(directory)
        verify_saved_assets(assets, source_options, scratch)
        provenance = {key: selection[key] for key in ('run_id', 'run_attempt', 'artifact_id', 'artifact_name')}
        provenance['run_url'] = (f'https://github.com/{selection["repository"]}/actions/runs/'
                                 f'{selection["run_id"]}')
        provenance['evidence_sha256'] = digest(assets / 'validation-evidence.zip')
        manifest, notes = prepare(scratch / 'inputs', output, selection['source_revision'],
                                  selection['release_tag'], selection['version'], built_from=provenance)
    for package in manifest['packages']:
        if digest(output / package['asset']) != digest(assets / package['asset']):
            raise ValueError('Saved platform package changed during release preparation')
    return manifest, notes


def ensure_tag(api, manifest):
    tag = manifest['tag']
    revision = manifest['source_revision']
    source = manifest['built_from']
    message = (f'Publish saved build {source["run_id"]} as {tag}\n'
               f'Source commit: {revision}\nArtifact: {source["artifact_id"]}\n')
    reference = api.get(tag_path(tag), missing=True)
    if reference is not None:
        item = reference['object']
        if item['type'] == 'tag':
            annotated = api.get('/git/tags/' + item['sha'])
            if (annotated.get('tag') == tag and annotated.get('message') == message
                    and annotated.get('object', {}).get('type') == 'commit'
                    and annotated['object'].get('sha') == revision):
                return  # Resume this exact publication after a failed draft upload.
        raise ValueError('Release tag already exists and was not created by this publication')
    annotated = api.post('/git/tags', {'tag': tag, 'message': message, 'object': revision, 'type': 'commit'})
    if not SHA.fullmatch(annotated.get('sha', '')):
        raise ValueError('Invalid created tag object')
    api.post('/git/refs', {'ref': 'refs/tags/' + tag, 'sha': annotated['sha']})


def publish_build(api, selection, manifest, output, notes, environment, publisher=publish):
    require_dispatch(environment)
    if environment.get('GITHUB_REPOSITORY') != selection['repository']:
        raise ValueError('Selected build belongs to another repository')
    recheck(api, selection)
    existing = api.get('/releases/tags/' + urllib.parse.quote(manifest['tag'], safe=''), missing=True)
    if existing is not None and not existing.get('draft'):
        raise ValueError('This release is already published; it will not be replaced')
    extra = environment.get('SAVED_BUILD_NOTES', '')
    if extra:
        notes.write_bytes(('\n'.join(extra.splitlines()) + '\n\n' + notes.read_text(encoding='utf-8')).encode('utf-8'))
    ensure_tag(api, manifest)
    return publisher(selection['repository'], manifest, output, notes)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('select', 'publish'))
    parser.add_argument('--selection', type=Path, required=True)
    parser.add_argument('--assets', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    try:
        require_dispatch(os.environ)
        repository = os.environ['GITHUB_REPOSITORY']
        api = GitHub(repository, os.environ['GH_TOKEN'])
        if args.command == 'select':
            requested = os.environ.get('SAVED_BUILD_RUN_ID', '').strip()
            selection = select_build(api, repository, os.environ['SAVED_BUILD_RELEASE_TAG'],
                                     int(requested) if requested else None)
            args.selection.parent.mkdir(parents=True, exist_ok=True)
            args.selection.write_bytes((json.dumps(selection, indent=2) + '\n').encode('utf-8'))
            with Path(os.environ['GITHUB_OUTPUT']).open('a', encoding='utf-8', newline='\n') as output:
                output.write(f'run_id={selection["run_id"]}\nartifact_id={selection["artifact_id"]}\n')
            print(json.dumps(selection, indent=2))
        else:
            if args.assets is None or args.output is None:
                parser.error('publish requires --assets and --output')
            selection = json.loads(args.selection.read_bytes())
            manifest, notes = prepare_saved(api, selection, args.assets, args.output)
            result = publish_build(api, selection, manifest, args.output, notes, os.environ)
            url = json.loads(result)['url']
            with Path(os.environ['GITHUB_STEP_SUMMARY']).open('a', encoding='utf-8', newline='\n') as summary:
                summary.write(f'## Release published\n\n[Open release]({url})\n\n'
                              f'Source commit: `{selection["source_revision"]}`\n\n'
                              f'[Original build]({manifest["built_from"]["run_url"]})\n\n'
                              'Platform packages reused without rebuilding.\n')
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        parser.exit(1, 'Saved build publication failed: ' + str(error) + '\n')


if __name__ == '__main__':
    main()
