#!/usr/bin/env python3
"""Verify a published validation candidate and promote its archives without builds."""
import argparse
import base64
import hashlib
import json
import os
import re
import shutil
import stat
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath

from publish_release import TARGETS, digest, prepare, publish, release_kind

JSON_LIMIT = 8 * 1024 * 1024
EVIDENCE_LIMIT = 64 * 1024 * 1024
UNPACKED_LIMIT = 256 * 1024 * 1024
PACKAGE_LIMIT = 512 * 1024 * 1024
SHA = re.compile(r'[0-9a-f]{40}')
HASH = re.compile(r'[0-9a-f]{64}')
WORKFLOW = '.github/workflows/native-validation.yml'


class SafeRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        if urllib.parse.urlsplit(new_url).scheme != 'https':
            raise ValueError('Release downloads require HTTPS redirects')
        redirected = super().redirect_request(request, fp, code, message, headers, new_url)
        if redirected and urllib.parse.urlsplit(request.full_url).netloc != urllib.parse.urlsplit(new_url).netloc:
            redirected.remove_header('Authorization')
        return redirected


class GitHub:
    def __init__(self, repository, token=None):
        if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository):
            raise ValueError('Invalid repository name')
        self.base = 'https://api.github.com/repos/' + repository
        self.token = token
        self.opener = urllib.request.build_opener(SafeRedirect())

    def request(self, path, data=None, binary=False):
        headers = {'Accept': 'application/octet-stream' if binary else 'application/vnd.github+json',
                   'User-Agent': 'Scripta-release-promotion'}
        if self.token:
            headers['Authorization'] = 'Bearer ' + self.token
        body = None if data is None else json.dumps(data).encode('utf-8')
        if body is not None:
            headers['Content-Type'] = 'application/json'
        return urllib.request.Request(self.base + path, data=body, headers=headers)

    def get(self, path, missing=False):
        try:
            with self.opener.open(self.request(path), timeout=30) as response:
                body = response.read(JSON_LIMIT + 1)
        except urllib.error.HTTPError as error:
            if missing and error.code == 404:
                return None
            raise
        if len(body) > JSON_LIMIT:
            raise ValueError('GitHub JSON response exceeds 8 MiB')
        return json.loads(body)

    def post(self, path, data):
        if not self.token:
            raise ValueError('Tag creation requires the workflow GITHUB_TOKEN')
        with self.opener.open(self.request(path, data), timeout=30) as response:
            body = response.read(JSON_LIMIT + 1)
        if len(body) > JSON_LIMIT:
            raise ValueError('GitHub JSON response exceeds 8 MiB')
        return json.loads(body)

    def download(self, asset, destination, limit):
        size = asset['size']
        if not isinstance(size, int) or not 0 < size <= limit:
            raise ValueError('Release asset size is outside promotion limits')
        total = 0
        with self.opener.open(self.request('/releases/assets/' + str(asset['id']), binary=True), timeout=60) as response:
            with destination.open('xb') as output:
                while True:
                    chunk = response.read(65536)
                    if not chunk:
                        break
                    total += len(chunk)
                    if total > size:
                        raise ValueError('Release asset exceeds its declared size')
                    output.write(chunk)
        if total != size:
            raise ValueError('Release asset size differs from GitHub metadata')


def tag_path(tag):
    if not tag or tag.startswith('-') or any(ord(c) < 32 for c in tag):
        raise ValueError('Invalid tag')
    return '/git/ref/tags/' + urllib.parse.quote(tag, safe='')


def resolve_tag(api, tag):
    reference = api.get(tag_path(tag))
    item = reference['object']
    for _ in range(8):
        if not SHA.fullmatch(item.get('sha', '')):
            raise ValueError('Invalid tag object SHA')
        if item['type'] == 'commit':
            return item['sha']
        if item['type'] != 'tag':
            break
        item = api.get('/git/tags/' + item['sha'])['object']
    raise ValueError('Tag does not resolve to a commit within eight objects')


def workspace_version(api, revision):
    content = api.get('/contents/Cargo.toml?ref=' + revision)
    if content.get('encoding') != 'base64' or content.get('size', JSON_LIMIT) > 65536:
        raise ValueError('Invalid validated workspace manifest')
    text = base64.b64decode(''.join(content['content'].split()), validate=True).decode('utf-8')
    section = re.search(r'^\[workspace\.package\]\s*\n(.*?)(?=^\[|\Z)', text, re.M | re.S)
    version = re.search(r'^version\s*=\s*"(\d+\.\d+\.\d+)"\s*$', section[1], re.M) if section else None
    if version is None:
        raise ValueError('Validated source must declare a stable workspace version')
    return version[1]


def validated_run(api, repository, run_id, source_tag, revision):
    if not isinstance(run_id, int) or run_id <= 0:
        raise ValueError('Validation run ID must be positive')
    run = api.get('/actions/runs/' + str(run_id))
    if (run.get('id') != run_id or run.get('event') != 'push' or run.get('path') != WORKFLOW
            or run.get('head_sha') != revision or run.get('head_branch') != source_tag
            or run.get('status') != 'completed' or run.get('conclusion') != 'success'
            or run.get('repository', {}).get('full_name', '').lower() != repository.lower()
            or run.get('head_repository', {}).get('full_name', '').lower() != repository.lower()):
        raise ValueError('Source run must be successful native validation of this exact tag and commit')
    jobs = api.get('/actions/runs/' + str(run_id) + '/jobs?per_page=100')
    expected = set(TARGETS) | {'Publish release downloads'}
    if (jobs.get('total_count') != len(expected) or len(jobs.get('jobs', [])) != len(expected)
            or {job.get('name') for job in jobs['jobs']} != expected
            or any(job.get('status') != 'completed' or job.get('conclusion') != 'success' for job in jobs['jobs'])):
        raise ValueError('All four native jobs and candidate publication must have passed')
    return run


def package_names(version):
    return {target: f'scripta-{version}-{target}' + ('.zip' if target.endswith('windows-msvc') else '.tar.gz')
            for target in TARGETS}


def checked_evidence(source, source_tag, revision, version, destination):
    names = package_names(version)
    assets = set(names.values())
    checksums = {}
    for line in (source / 'SHA256SUMS').read_bytes().decode('ascii').splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  ([A-Za-z0-9._-]+)', line)
        if not match or match[2] in checksums:
            raise ValueError('Malformed or duplicate source checksums')
        checksums[match[2]] = match[1]
    if set(checksums) != assets | {'validation-evidence.zip'}:
        raise ValueError('Source checksums must cover exactly the four packages and evidence ZIP')
    for name, checksum in checksums.items():
        if digest(source / name) != checksum:
            raise ValueError('Source asset checksum mismatch: ' + name)

    unpacked = destination / 'evidence'
    unpacked.mkdir()
    with zipfile.ZipFile(source / 'validation-evidence.zip') as bundle:
        infos = bundle.infolist()
        if len(infos) > 512 or len({info.filename for info in infos}) != len(infos):
            raise ValueError('Evidence ZIP has duplicate or excessive entries')
        if sum(info.file_size for info in infos) > UNPACKED_LIMIT:
            raise ValueError('Unpacked evidence exceeds 256 MiB')
        required = {'release-manifest.json'} | {f'validation-{target}.json' for target in TARGETS}
        required |= {name + suffix for name in assets for suffix in ('.sha256', '.validation.json')}
        for info in infos:
            name = info.filename
            path = PurePosixPath(name)
            log = any(re.fullmatch(re.escape('logs/' + target + '/') + r'[A-Za-z0-9_.-]+\.log', name)
                      for target in TARGETS)
            if ((name not in required and not log) or '\\' in name or path.is_absolute()
                    or '..' in path.parts or path.as_posix() != name or info.is_dir()
                    or stat.S_IFMT(info.external_attr >> 16) not in (0, stat.S_IFREG)
                    or info.file_size > (JSON_LIMIT if name.endswith('.json') else EVIDENCE_LIMIT)):
                raise ValueError('Unexpected or unsafe evidence ZIP entry: ' + name)
        if not required.issubset({info.filename for info in infos}):
            raise ValueError('Evidence ZIP is incomplete')
        # Every path is whitelisted before any member is extracted. Bounded copy
        # also protects against forged ZIP sizes; original evidence stays raw.
        for info in infos:
            output = unpacked.joinpath(*PurePosixPath(info.filename).parts)
            output.parent.mkdir(parents=True, exist_ok=True)
            total = 0
            with bundle.open(info) as stream, output.open('xb') as target:
                while True:
                    chunk = stream.read(65536)
                    if not chunk:
                        break
                    total += len(chunk)
                    if total > info.file_size:
                        raise ValueError('Evidence entry exceeds its declared size')
                    target.write(chunk)

    manifest = json.loads((unpacked / 'release-manifest.json').read_bytes())
    if (manifest.get('tag') != source_tag or manifest.get('source_revision') != revision
            or manifest.get('version') != version or manifest.get('prerelease') is not True):
        raise ValueError('Candidate manifest does not match validated source')
    packages = manifest.get('packages', [])
    if len(packages) != 4 or {p.get('asset') for p in packages} != assets:
        raise ValueError('Candidate manifest must describe all four platform packages')
    for package in packages:
        if (names.get(package.get('target')) != package['asset'] or package.get('variant') != 'standard'
                or package.get('extensions') != ['sqlite']
                or package.get('archive_sha256') != checksums[package['asset']]):
            raise ValueError('Candidate package metadata differs from its archive')
    files = manifest.get('files', {})
    members = {p.relative_to(unpacked).as_posix() for p in unpacked.rglob('*') if p.is_file()}
    if set(files) != assets | (members - {'release-manifest.json'}):
        raise ValueError('Candidate evidence inventory differs')
    for name, checksum in files.items():
        if not isinstance(checksum, str) or not HASH.fullmatch(checksum):
            raise ValueError('Invalid manifest checksum')
        path = source / name if name in assets else unpacked / name
        if digest(path) != checksum:
            raise ValueError('Candidate evidence checksum mismatch: ' + name)
    for name in assets:
        if (unpacked / (name + '.sha256')).read_bytes() != f'{checksums[name]}  {name}\n'.encode('ascii'):
            raise ValueError('Individual package checksum differs')

    inputs = destination / 'inputs'
    for target, name in names.items():
        directory = inputs / target
        (directory / 'standard').mkdir(parents=True)
        shutil.copyfile(unpacked / f'validation-{target}.json', directory / 'validation.json')
        shutil.copyfile(source / name, directory / 'standard' / name)
        shutil.copyfile(unpacked / (name + '.validation.json'), directory / 'standard' / (name + '.validation.json'))
        if (unpacked / 'logs' / target).is_dir():
            shutil.copytree(unpacked / 'logs' / target, directory / 'logs')
    return inputs


def prepare_promotion(api, repository, source_tag, run_id, release_tag, output):
    if output.exists():
        raise ValueError('Choose a fresh promotion output directory')
    revision = resolve_tag(api, source_tag)
    version = workspace_version(api, revision)
    if release_tag != 'v' + version or not release_kind(source_tag, version):
        raise ValueError('Promote a validation/prerelease tag to the exact stable workspace version')
    run = validated_run(api, repository, run_id, source_tag, revision)
    source_release = api.get('/releases/tags/' + urllib.parse.quote(source_tag, safe=''))
    if (source_release.get('tag_name') != source_tag or source_release.get('draft')
            or source_release.get('prerelease') is not True):
        raise ValueError('Source must be a published prerelease')
    existing = api.get('/releases/tags/' + release_tag, missing=True)
    if existing is not None and not existing.get('draft'):
        raise ValueError('Stable release is already published; it will not be replaced')
    assets = source_release['assets']
    expected = set(package_names(version).values()) | {'SHA256SUMS', 'validation-evidence.zip'}
    if len(assets) != 6 or {asset['name'] for asset in assets} != expected:
        raise ValueError('Source prerelease must contain exactly the six release assets')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='scripta-promotion-', dir=output.parent) as directory:
        scratch = Path(directory)
        source = scratch / 'source'
        source.mkdir()
        for asset in assets:
            limit = 65536 if asset['name'] == 'SHA256SUMS' else EVIDENCE_LIMIT if asset['name'] == 'validation-evidence.zip' else PACKAGE_LIMIT
            api.download(asset, source / asset['name'], limit)
        inputs = checked_evidence(source, source_tag, revision, version, scratch)
        provenance = {'tag': source_tag, 'run_id': run_id, 'run_url': run['html_url'],
                      'run_attempt': run.get('run_attempt', 1), 'release_url': source_release['html_url'],
                      'evidence_sha256': digest(source / 'validation-evidence.zip')}
        manifest, notes = prepare(inputs, output, revision, release_tag, version, promoted_from=provenance)
        for package in manifest['packages']:
            if digest(output / package['asset']) != digest(source / package['asset']):
                raise ValueError('Promotion changed a validated platform archive')
    return manifest, notes


def promotion_message(manifest):
    source = manifest['promoted_from']
    return (f"Promote {manifest['tag']} from {source['tag']}\n"
            f"Validated commit: {manifest['source_revision']}\nValidation run: {source['run_id']}\n")


def ensure_promotion_tag(api, manifest):
    tag = manifest['tag']
    revision = manifest['source_revision']
    message = promotion_message(manifest)
    reference = api.get(tag_path(tag), missing=True)
    if reference is not None:
        item = reference['object']
        if item['type'] != 'tag':
            raise ValueError('Existing stable tag was not created by this promotion')
        annotated = api.get('/git/tags/' + item['sha'])
        if (annotated.get('tag') != tag or annotated.get('message') != message
                or annotated.get('object', {}).get('type') != 'commit'
                or annotated.get('object', {}).get('sha') != revision):
            raise ValueError('Existing stable tag differs from this promotion; it will not be moved')
        return
    annotated = api.post('/git/tags', {'tag': tag, 'message': message, 'object': revision, 'type': 'commit'})
    if not SHA.fullmatch(annotated.get('sha', '')):
        raise ValueError('Invalid created tag object')
    api.post('/git/refs', {'ref': 'refs/tags/' + tag, 'sha': annotated['sha']})


def require_dispatch(environment):
    if (environment.get('GITHUB_ACTIONS') != 'true' or environment.get('GITHUB_EVENT_NAME') != 'workflow_dispatch'
            or not environment.get('PROMOTION_DEFAULT_BRANCH')
            or environment.get('GITHUB_REF') != 'refs/heads/' + environment['PROMOTION_DEFAULT_BRANCH']
            or not environment.get('GH_TOKEN')):
        raise ValueError('Publishing requires manual dispatch on the default branch using its GITHUB_TOKEN')


def promote(api, repository, manifest, output, notes, environment, publisher=publish):
    require_dispatch(environment)
    source = manifest['promoted_from']
    if resolve_tag(api, source['tag']) != manifest['source_revision']:
        raise ValueError('Validation tag changed during preparation')
    run = validated_run(api, repository, source['run_id'], source['tag'], manifest['source_revision'])
    if run.get('run_attempt', 1) != source['run_attempt']:
        raise ValueError('Validation run was retried during preparation')
    existing = api.get('/releases/tags/' + manifest['tag'], missing=True)
    if existing is not None and not existing.get('draft'):
        raise ValueError('Stable release was published during preparation; it will not be replaced')
    ensure_promotion_tag(api, manifest)
    if resolve_tag(api, manifest['tag']) != manifest['source_revision']:
        raise ValueError('Stable tag does not point to the validated commit')
    return publisher(repository, manifest, output, notes)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', default=os.environ.get('GITHUB_REPOSITORY'))
    parser.add_argument('--validation-tag', required=True)
    parser.add_argument('--validation-run-id', type=int, required=True)
    parser.add_argument('--release-tag', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--publish', action='store_true', help='Create the stable tag and publish; default is read-only preparation')
    args = parser.parse_args()
    if args.publish:
        require_dispatch(os.environ)
    api = GitHub(args.repository or '', os.environ.get('GH_TOKEN') or os.environ.get('GITHUB_TOKEN'))
    manifest, notes = prepare_promotion(api, args.repository, args.validation_tag, args.validation_run_id, args.release_tag, args.output)
    print(json.dumps({'tag': manifest['tag'], 'source_revision': manifest['source_revision'],
                      'promoted_from': manifest['promoted_from'], 'packages': manifest['packages']}, indent=2))
    if args.publish:
        print(promote(api, args.repository, manifest, args.output, notes, os.environ))


if __name__ == '__main__':
    main()
