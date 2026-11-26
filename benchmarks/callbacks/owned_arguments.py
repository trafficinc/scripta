#!/usr/bin/env python3
"""Compare preserved before/after Scripta executables without cross-language noise."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import platform
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

from run import ENV, ROOT, fixture, identity


def checked(command):
    return subprocess.run(command, cwd=ROOT, env=ENV, check=True, capture_output=True,
                          text=True, timeout=300).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--candidate', type=Path)
    parser.add_argument('--baseline-cli', type=Path)
    parser.add_argument('--candidate-cli', type=Path)
    parser.add_argument('--rounds', type=int, default=4)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--source', action='append', default=[], help='additional source path to hash')
    args = parser.parse_args()
    if args.rounds < 1 or bool(args.baseline_cli) != bool(args.candidate_cli):
        parser.error('positive rounds and either both CLI paths or neither required')
    args.output.mkdir(parents=True, exist_ok=False)
    data_path = args.output.resolve() / 'fixture.json'
    data_path.write_text(json.dumps(fixture([128, 2048, 8192]), ensure_ascii=False), encoding='utf-8')
    binaries = {'baseline': args.baseline}
    if args.candidate:
        binaries['candidate'] = args.candidate
    report = {
        'started_utc': datetime.now(timezone.utc).isoformat(),
        'revision': checked(['git', 'rev-parse', 'HEAD']).strip(),
        'working_tree': checked(['git', 'status', '--short']),
        'compiler': checked(['rustc', '-vV']),
        'host': platform.platform(), 'machine': platform.machine(),
        'settings': {'sizes': [128, 2048, 8192], 'samples': 11, 'warmup_ms': 100,
                     'target_batch_ms': 10, 'max_batch': 10000,
                     'startup_warmups': 3, 'startup_samples': 15,
                     'scripta_profile': 'release', 'deployment_target': '15.0'},
        'binaries': {key: identity(path) for key, path in binaries.items()},
        'sources': [identity(ROOT / name) for name in [
            'benchmarks/callbacks/owned_arguments.py', 'benchmarks/callbacks/run.py',
            'crates/scr-stdlib/benches/callback_comparison.rs',
            'crates/scr-runtime/src/invocation.rs', 'crates/scr-stdlib/src/collections/callbacks.rs', *args.source]],
        'fixture_sha256': hashlib.sha256(data_path.read_bytes()).hexdigest(),
        'rounds': args.rounds, 'runs': [], 'startup_ms': {},
        'cli_control_ms': {}, 'cli_control_fixtures': [],
    }
    (args.output / 'working-tree.patch').write_text(checked(['git', 'diff']), encoding='utf-8')
    names = list(binaries)
    for round_index in range(args.rounds):
        order = names if round_index % 2 == 0 else list(reversed(names))
        for name in order:
            print(f'Round {round_index+1}/{args.rounds}: {name}', flush=True)
            raw = checked([str(binaries[name]), str(data_path)])
            parsed = json.loads(raw)
            assert parsed['runtime'] == 'scripta'
            for row in parsed['results']:
                assert len(row['samples_ms']) == 11
                assert all(math.isfinite(t) and t > 0 for t in row['samples_ms'])
            (args.output / f'{round_index+1}-{name}.json').write_text(raw, encoding='utf-8')
            report['runs'].append({'round': round_index+1, 'variant': name, **parsed})
    if args.baseline_cli:
        clis = {'baseline': args.baseline_cli, 'candidate': args.candidate_cli}
        report['cli_binaries'] = {name: identity(path) for name, path in clis.items()}
        report['startup_ms'] = {name: [] for name in clis}
        with tempfile.TemporaryDirectory(prefix='scripta-owned-startup-') as temporary:
            empty = Path(temporary) / 'empty.scripta'
            empty.write_text('', encoding='utf-8')
            for sample in range(18):
                order = list(clis) if sample % 2 == 0 else list(reversed(clis))
                for name in order:
                    start = time.perf_counter_ns()
                    result = checked([str(clis[name]), 'run', str(empty)])
                    elapsed = (time.perf_counter_ns()-start)/1e6
                    assert result == ''
                    if sample >= 3:
                        report['startup_ms'][name].append(elapsed)
        controls = {
            'numeric_loop': '143997165\n',
            'function_calls': '20000100000\n',
            'list_pipeline': '200000:599400000\n',
        }
        for workload, expected in controls.items():
            source = ROOT / 'benchmarks/comparison/fixtures' / (workload + '.scripta')
            report['cli_control_fixtures'].append(identity(source))
            report['cli_control_ms'][workload] = {name: [] for name in clis}
            for sample in range(18):
                order = list(clis) if sample % 2 == 0 else list(reversed(clis))
                for name in order:
                    start = time.perf_counter_ns()
                    actual = checked([str(clis[name]), 'run', str(source)])
                    elapsed = (time.perf_counter_ns()-start)/1e6
                    assert actual.replace('\r\n', '\n') == expected, workload
                    if sample >= 3:
                        report['cli_control_ms'][workload][name].append(elapsed)
    summary = []
    for row in report['runs'][0]['results']:
        entry = {'operation': row['operation'], 'size': row['size']}
        for name in names:
            medians = [statistics.median(r['samples_ms']) for run in report['runs']
                       if run['variant'] == name for r in run['results']
                       if (r['operation'], r['size']) == (row['operation'], row['size'])]
            entry[name] = {'median_ms': statistics.median(medians), 'run_medians_ms': medians}
        if args.candidate:
            entry['speedup'] = entry['baseline']['median_ms']/entry['candidate']['median_ms']
        summary.append(entry)
    report['summary'] = summary
    report['finished_utc'] = datetime.now(timezone.utc).isoformat()
    (args.output / 'evidence.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    print('size,operation,' + ','.join(names))
    for row in summary:
        print(f"{row['size']},{row['operation']}," + ','.join(f"{row[name]['median_ms']:.6f}" for name in names))
    if report['startup_ms']:
        print('Startup medians (ms):', {k: statistics.median(v) for k, v in report['startup_ms'].items()})
    for workload, observations in report['cli_control_ms'].items():
        print('CLI control', workload, {k: statistics.median(v) for k, v in observations.items()})


if __name__ == '__main__':
    main()
