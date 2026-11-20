#!/usr/bin/env python3
"""Scan current files and Git history locally with Gitleaks; retain locations only."""
import argparse
from collections import Counter
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / '.gitleaks.toml'


def locations(findings, scope, root):
    """Whitelist fields: raw matches, secrets, messages, authors never leave here."""
    result = []
    for finding in findings:
        path = Path(finding['File'])
        if path.is_absolute():
            path = path.relative_to(root)
        result.append({
            'scope': scope,
            'rule': finding['RuleID'],
            'file': path.as_posix(),
            'line': finding['StartLine'],
            'commit': finding.get('Commit', ''),
        })
    return result


def scan(binary, root, scopes):
    version = subprocess.run([binary, 'version'], capture_output=True, text=True,
                             check=True, timeout=15).stdout.strip()
    match = re.search(r'\b(\d+)\.(\d+)\.(\d+)\b', version)
    if not match or tuple(map(int, match.groups())) < (8, 25, 0):
        raise ValueError('Gitleaks 8.25.0 or newer is required (tested with 8.30.1).')
    findings = []
    if 'history' in scopes:
        repository = subprocess.run(['git', '-C', str(root), 'rev-parse', '--show-toplevel'],
                                    capture_output=True, text=True, timeout=30)
        if repository.returncode or Path(repository.stdout.strip()).resolve() != root:
            raise ValueError('History scan requires the Git repository root.')
        subprocess.run(['git', '-C', str(root), 'rev-parse', '--verify', 'HEAD'],
                       capture_output=True, check=True, timeout=30)
    # Empty ignore file prevents local fingerprint suppressions from hiding results.
    with tempfile.TemporaryDirectory(prefix='scripta-sensitive-') as directory:
        ignore = Path(directory) / 'empty.gitleaksignore'
        ignore.touch()
        for scope in scopes:
            command = [binary, 'dir' if scope == 'files' else 'git', str(root),
                       '--config', str(CONFIG), '--redact=100', '--no-banner',
                       '--no-color', '--log-level', 'error',
                       '--report-format', 'json', '--report-path', '-',
                       '--exit-code', '10', '--ignore-gitleaks-allow',
                       '--gitleaks-ignore-path', str(ignore), '--timeout', '600']
            if scope == 'history':
                command += ['--log-opts=--all']
            completed = subprocess.run(command, cwd=root, capture_output=True,
                                       text=True, timeout=630)
            if completed.returncode not in (0, 10) or completed.stderr.strip():
                # Do not echo scanner logs: they can contain source or personal data.
                raise ValueError(f'{scope} scan failed (exit {completed.returncode}); '
                                 'check Gitleaks configuration and Git access.')
            raw = json.loads(completed.stdout)
            if not isinstance(raw, list):
                raise ValueError(f'{scope} scan returned an invalid report.')
            findings.extend(locations(raw, scope, root))
    return version, findings


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=ROOT, help='directory/repository to scan')
    parser.add_argument('--scope', choices=('all', 'files', 'history'), default='all')
    local_scanner = ROOT / '.security-tools' / ('gitleaks.exe' if os.name == 'nt' else 'gitleaks')
    default_scanner = shutil.which('gitleaks') or str(local_scanner)
    parser.add_argument('--gitleaks', default=os.environ.get('GITLEAKS_BIN', default_scanner),
                        help='installed executable, or set GITLEAKS_BIN')
    args = parser.parse_args(argv)
    binary = shutil.which(args.gitleaks)
    if binary is None:
        print('Gitleaks not found. Install it outside the repo (macOS: brew install '
              'gitleaks), or pass --gitleaks /path/to/gitleaks.', file=sys.stderr)
        return 2
    root = args.root.resolve()
    scopes = ['files', 'history'] if args.scope == 'all' else [args.scope]
    try:
        version, findings = scan(binary, root, scopes)
        report_dir = root / '.security-reports'
        report_dir.mkdir(mode=0o700, exist_ok=True)
        # Replace atomically so a failed run cannot leave a partial successful report.
        with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8', newline='\n',
                                         dir=report_dir, delete=False) as output:
            temporary = Path(output.name)
            try:
                json.dump({'scanner': version, 'scopes': scopes, 'findings': findings},
                          output, indent=2, ensure_ascii=True)
                output.write('\n')
                output.close()
                temporary.replace(report_dir / 'findings.json')
            finally:
                temporary.unlink(missing_ok=True)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        print('Scan failed; no new report was written. Check executable version '
              '(8.25+), configuration, directory permissions, and Git history access. '
              'Any existing report belongs to an earlier run.', file=sys.stderr)
        return 2
    print(f'Gitleaks {version}: {len(findings)} potential findings.')
    for rule, count in sorted(Counter(item['rule'] for item in findings).items()):
        print(f'  {rule}: {count}')
    print('Locations only: .security-reports/findings.json under the scan root.')
    return 1 if findings else 0


if __name__ == '__main__':
    sys.exit(main())
