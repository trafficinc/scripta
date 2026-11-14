#!/usr/bin/env python3
"""Native test/package evidence for tag-triggered CI; Python standard library only."""
import argparse
import hashlib
import json
import os
import platform
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGETS = (
    'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu',
    'x86_64-pc-windows-msvc', 'aarch64-apple-darwin',
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def test_counts(text):
    rows = re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', text)
    if not rows or any(row[0] != 'ok' or row[2:] != ('0', '0') for row in rows):
        raise RuntimeError('Test suite is missing results, failed, or ignored tests')
    return {'passed': sum(int(row[1]) for row in rows), 'failed': 0, 'ignored': 0}


def windows_imports(text):
    imports = re.findall(r'^\s+([\w.-]+\.dll)\s*$', text, re.M | re.I)
    if not imports:
        raise RuntimeError('dumpbin reported no DLL imports')
    forbidden = re.compile(r'^(vcruntime|msvcp|msvcr|ucrtbase|api-ms-win-crt-|libssl|libcrypto|sqlite3)', re.I)
    if any(forbidden.match(name) for name in imports):
        raise RuntimeError('Unexpected runtime DLL dependency: ' + ', '.join(imports))
    return sorted(set(imports), key=str.lower)


class Validation:
    def __init__(self, target, output):
        self.target = target
        self.output = output
        self.logs = output / 'logs'
        self.logs.mkdir(parents=True, exist_ok=True)
        self.environment = os.environ.copy()
        self.environment['CARGO_TERM_COLOR'] = 'never'
        if target.endswith('apple-darwin'):
            self.environment['MACOSX_DEPLOYMENT_TARGET'] = '15.0'
        self.record = {
            'target': target, 'host': platform.platform(), 'status': 'running',
            'steps': [], 'tests': {}, 'packages': {},
            'support_status': 'requires maintainer review of native evidence; not inferred automatically',
        }

    def run(self, label, arguments):
        arguments = [str(item) for item in arguments]
        print('Starting ' + label, flush=True)
        path = self.logs / (label + '.log')
        with path.open('w', encoding='utf-8') as log:
            log.write(json.dumps(arguments) + '\n')
            log.flush()
            try:
                result = subprocess.run(arguments, cwd=ROOT, env=self.environment,
                                        stdout=log, stderr=subprocess.STDOUT)
            except OSError as error:
                log.write(str(error) + '\n')
                self.record['steps'].append({'name': label, 'command': arguments,
                                             'exit_code': None, 'error': str(error),
                                             'log': str(path.relative_to(self.output))})
                self.save()
                raise RuntimeError(label + ' could not start: ' + str(error)) from error
        text = path.read_text(encoding='utf-8', errors='replace')
        self.record['steps'].append({'name': label, 'command': arguments,
                                     'exit_code': result.returncode, 'log': str(path.relative_to(self.output))})
        self.save()
        if result.returncode:
            print(text[-12000:], flush=True)
            raise RuntimeError(label + ' failed; see ' + str(path))
        print('Passed ' + label, flush=True)
        return text.split('\n', 1)[1]

    def save(self):
        (self.output / 'validation.json').write_text(json.dumps(self.record, indent=2) + '\n', encoding='utf-8')

    def cargo(self, label, arguments):
        return self.run(label, ['cargo', *arguments, '--locked', '--target', self.target])

    def dependencies(self, variant, binary):
        if self.target.endswith('windows-msvc'):
            installer = Path(os.environ.get('ProgramFiles(x86)', 'C:/Program Files (x86)')) / 'Microsoft Visual Studio/Installer/vswhere.exe'
            locations = self.run('find-dumpbin-' + variant, [installer, '-latest', '-products', '*',
                                 '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
                                 '-find', 'VC/Tools/MSVC/**/bin/Hostx64/x64/dumpbin.exe']).splitlines()
            if not locations:
                raise RuntimeError('Visual Studio dumpbin was not found')
            text = self.run('dependencies-' + variant, [locations[-1], '/dependents', binary])
            return windows_imports(text)
        if self.target.endswith('linux-gnu'):
            text = self.run('dependencies-' + variant, ['ldd', binary])
            if 'not found' in text:
                raise RuntimeError('Missing Linux shared library')
            allowed = re.compile(r'^(linux-vdso\.so\.1|ld-linux[\w.-]*\.so(?:\.\d+)*|lib(?:gcc_s|c|m|dl|pthread|rt|resolv|util)\.so(?:\.\d+)*)$')
            libraries = []
            for line in text.splitlines():
                if not line.strip():
                    continue
                name = line.strip().split()[0]
                if not allowed.fullmatch(Path(name).name):
                    raise RuntimeError('Unexpected Linux shared library: ' + name)
                libraries.append(line.strip())
            if not libraries:
                raise RuntimeError('No Linux dependency evidence')
            return libraries
        text = self.run('dependencies-' + variant, ['otool', '-L', binary])
        libraries = [line.strip().split(' (', 1)[0] for line in text.splitlines()[1:]]
        if not libraries or any(not name.startswith(('/usr/lib/', '/System/Library/')) for name in libraries):
            raise RuntimeError('Unexpected macOS shared library')
        return libraries

    def execute(self):
        channel = re.search(r'^channel\s*=\s*"([^"]+)"', (ROOT / 'rust-toolchain.toml').read_text(), re.M)
        if not channel:
            raise RuntimeError('Missing pinned Rust channel')
        self.run('toolchain', ['rustup', 'toolchain', 'install', channel[1], '--profile', 'minimal',
                              '--component', 'rustfmt', '--component', 'clippy', '--target', self.target])
        compiler = self.run('compiler', ['rustc', '--version', '--verbose'])
        if not re.search(r'^host: ' + re.escape(self.target) + r'\s*$', compiler, re.M):
            raise RuntimeError('Compiler host differs from native target')
        self.record['compiler'] = compiler
        revision = self.run('revision', ['git', 'rev-parse', 'HEAD']).strip()
        self.record['source_revision'] = revision
        self.environment['SCR_BUILD_COMMIT'] = revision
        self.record['source_date_epoch'] = self.run('epoch', ['git', 'show', '-s', '--format=%ct', 'HEAD']).strip()
        self.run('format', ['cargo', 'fmt', '--all', '--', '--check'])
        self.run('validation-script-tests', [sys.executable, '-m', 'unittest', 'discover', '-s', str(ROOT / 'scripts'), '-p', 'test_*.py'])
        for label, profile in [('debug', []), ('dist', ['--profile', 'dist'])]:
            for suffix, features in [('default', []), ('all-features', ['--all-features'])]:
                name = 'tests-' + label + '-' + suffix
                text = self.cargo(name, ['test', '--workspace', *profile, *features])
                self.record['tests'][name] = test_counts(text)
                self.save()
            name = 'tests-' + label + '-minimal'
            text = self.cargo(name, ['test', '-p', 'scr-cli', '--no-default-features',
                                     '--test', 'sqlite_extension', *profile])
            self.record['tests'][name] = test_counts(text)
            self.save()
        self.run('clippy', ['cargo', 'clippy', '--workspace', '--all-targets', '--all-features',
                           '--locked', '--target', self.target, '--', '-D', 'warnings'])
        self.cargo('build-release-tool', ['build', '-p', 'scr-release'])
        extension = '.exe' if self.target.endswith('windows-msvc') else ''
        release = ROOT / 'target' / self.target / 'debug' / ('scr-release' + extension)
        version = self.run('workspace-version', [release, 'version']).strip()
        self.record['version'] = version
        variant = 'standard'
        self.cargo('build-' + variant, ['build', '-p', 'scr-cli', '--profile', 'dist'])
        binary = ROOT / 'target' / self.target / 'dist' / ('scripta' + extension)
        description = self.run('version-' + variant, [binary, 'version', '--verbose'])
        expected = 'extensions: sqlite'
        if expected not in description.splitlines() or ('commit: ' + revision) not in description.splitlines():
            raise RuntimeError('Binary feature/revision metadata mismatch')
        dependencies = self.dependencies(variant, binary)
        destination = self.output / variant
        arguments = [release, 'package', '--binary', binary, '--target', self.target,
                     '--epoch', self.record['source_date_epoch']]
        self.run('package-' + variant, [*arguments, '--out-dir', destination])
        archive = destination / ('scripta-' + version + '-' + self.target + ('.zip' if extension else '.tar.gz'))
        self.run('verify-' + variant, [release, 'verify', '--archive', archive])
        self.run('smoke-' + variant, [release, 'smoke', '--archive', archive])
        self.run('checksums-' + variant, [release, 'checksums', '--dir', destination])
        self.run('verify-checksums-' + variant, [release, 'verify-checksums', '--dir', destination])
        with tempfile.TemporaryDirectory(prefix='scripta-repack-') as scratch:
            self.run('repack-' + variant, [*arguments, '--out-dir', scratch])
            if archive.read_bytes() != (Path(scratch) / archive.name).read_bytes():
                raise RuntimeError('Identical-input repacking changed the archive')
        self.record['packages'][variant] = {
            'archive': str(archive.relative_to(self.output)), 'archive_sha256': digest(archive),
            'binary_sha256': digest(binary), 'dependencies': dependencies,
            'extensions': ['sqlite'],
            'identical_input_repack': 'passed', 'offline_package_smoke': 'passed',
        }
        self.save()
        self.record['status'] = 'passed'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=TARGETS, required=True)
    parser.add_argument('--output', type=Path, required=True)
    arguments = parser.parse_args()
    output = arguments.output.resolve()
    if output.exists() and any(output.iterdir()):
        parser.error('Choose a fresh output directory to preserve earlier evidence')
    validation = Validation(arguments.target, output)
    try:
        validation.execute()
        if os.environ.get('GITHUB_ACTIONS') == 'true':
            evidence = {key: validation.record[key] for key in
                        ['target', 'source_revision', 'tests', 'packages']}
            message = json.dumps(evidence).replace('%', '%25')
            print('::notice title=Native validation evidence::' + message, flush=True)
    except Exception as error:
        validation.record['status'] = 'failed'
        validation.record['error'] = str(error)
        print(str(error), file=sys.stderr)
        if os.environ.get('GITHUB_ACTIONS') == 'true':
            details = str(error)[:500]
            if validation.record['steps']:
                log = validation.output / validation.record['steps'][-1]['log']
                # Check annotations truncate at 4096 characters. Keep the
                # final failure, rather than losing it behind preceding passes.
                details += '\n' + log.read_text(encoding='utf-8', errors='replace')[-3000:]
            # Preserve the useful error in public check annotations, even when
            # downloading the complete job log requires authentication.
            escaped = details.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
            print('::error title=Native validation failure::' + escaped, flush=True)
        return 1
    finally:
        validation.save()
    return 0


if __name__ == '__main__':
    sys.exit(main())
