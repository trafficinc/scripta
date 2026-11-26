#!/usr/bin/env python3
"""Run prebuilt engines sequentially, verify outputs, retain raw paired evidence."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
import shutil
import sys
import platform
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REMOVED_ENV = sorted(k for k in os.environ if k.startswith("SCR_") or k in {"NODE_OPTIONS", "NODE_PATH"})
ENV = {k: v for k, v in os.environ.items() if k not in REMOVED_ENV}
ENV["LC_ALL"] = "C"


def fixture(sizes):
    datasets = []
    for size in sizes:
        values = list(range(size))
        entries = [[i if i % 2 == 0 else str(i), i] for i in values]
        rows = [{"id": i, "rank": (i * 73) % 97,
                 "payload": ["x" * 128, "世界"]} for i in values]
        # Ordered maps serialize as ordered pairs to retain typed keys and ties.
        encoded_rows = [[[k, row[k]] for k in sorted(row)] for row in rows]
        sorted_rows = [encoded_rows[i] for i in sorted(values, key=lambda i: rows[i]["rank"])]
        expected = {
            "list_map": [i * i for i in values],
            "list_map_capture": [i + 7 for i in values],
            "list_filter": [i for i in values if i % 2 == 0],
            "list_reduce": sum(values),
            "map_map": [[k, v + 1] for k, v in entries],
            "map_filter": [[k, v] for k, v in entries if v % 2 == 0],
            "map_reduce": sum(values),
            "record_map": [row["rank"] for row in rows],
            "record_sort_key": sorted_rows,
            "record_sort_compare": sorted_rows,
            "reduce_growing_copy": values,
        }
        datasets.append(dict(size=size, values=values, entries=entries, rows=rows, expected=expected))
    return {"datasets": datasets}


def identity(path):
    resolved = Path(path).resolve()
    return {"path": str(resolved), "sha256": hashlib.sha256(resolved.read_bytes()).hexdigest()}


def checked(command):
    return subprocess.run(command, cwd=ROOT, check=True, capture_output=True,
                          text=True, env=ENV, timeout=300).stdout


def summarize(runs, sizes, runtimes):
    summary = []
    for size in sizes:
        names = list(dict.fromkeys(row['operation'] for run in runs
                                  for row in run['results'] if row['size'] == size))
        for name in names:
            row = {'size': size, 'operation': name}
            for runtime in runtimes:
                medians = [statistics.median(r['samples_ms']) for run in runs
                           if run['runtime'] == runtime for r in run['results']
                           if r['size'] == size and r['operation'] == name]
                if medians:
                    row[runtime] = {'median_ms': statistics.median(medians),
                                    'run_medians_ms': medians,
                                    'min_run_median_ms': min(medians),
                                    'max_run_median_ms': max(medians)}
            summary.append(row)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True,
                        help='prebuilt callback_comparison bench executable')
    parser.add_argument('--scripta', type=Path, required=True,
                        help='optimized CLI for startup measurement')
    parser.add_argument('--node', required=True)
    parser.add_argument('--php', required=True)
    parser.add_argument('--python', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--rounds', '--pairs', dest='rounds', type=int, default=4)
    parser.add_argument('--sizes', type=int, nargs='+', default=[128, 2048, 8192])
    args = parser.parse_args()
    if args.rounds < 1 or len(set(args.sizes)) != len(args.sizes) or any(n < 1 or n > 8192 for n in args.sizes):
        parser.error('rounds must be positive; sizes must be distinct and in 1..8192')
    executables = {}
    for name in ['node', 'php', 'python']:
        path = shutil.which(str(getattr(args, name)))
        if path is None:
            parser.error(f'executable not found: {getattr(args, name)}')
        executables[name] = path
    args.output.mkdir(parents=True, exist_ok=False)
    data = fixture(args.sizes)
    fixture_path = args.output.resolve() / 'fixture.json'
    fixture_path.write_text(json.dumps(data, ensure_ascii=False), encoding='utf-8')
    # No php.ini, CLI OPcache or PHP JIT. An explicit memory ceiling accommodates
    # decoding all reference outputs; other engines use their ordinary heap policy.
    php_flags = ['-n', '-d', 'opcache.enable_cli=0', '-d', 'opcache.jit=0',
                 '-d', 'memory_limit=512M']
    runtimes = ['scripta', 'node', 'php', 'python']
    commands = {
        'scripta': [str(args.engine), str(fixture_path)],
        'node': [executables['node'], str(ROOT / 'benchmarks/callbacks/node.cjs'), str(fixture_path)],
        'php': [executables['php'], *php_flags, str(ROOT / 'benchmarks/callbacks/php.php'), str(fixture_path)],
        'python': [executables['python'], '-I', str(ROOT / 'benchmarks/callbacks/python.py'), str(fixture_path)],
    }
    # PHP_BINARY identifies the actual interpreter behind a launcher script.
    php_binary = checked([executables['php'], *php_flags, '-r', 'echo PHP_BINARY;']).strip()
    sources = [Path(__file__), ROOT / 'crates/scr-stdlib/benches/callback_comparison.rs']
    sources += [ROOT / 'benchmarks/callbacks' / name for name in ['node.cjs', 'php.php', 'python.py']]
    binaries = [('scripta_engine', args.engine), ('scripta_cli', args.scripta),
                *executables.items(), ('php_interpreter', php_binary)]
    evidence = {
        'started_utc': datetime.now(timezone.utc).isoformat(),
        'revision': checked(['git', 'rev-parse', 'HEAD']).strip(),
        'working_tree': checked(['git', 'status', '--short']),
        'host': platform.platform(), 'machine': platform.machine(),
        'processor': platform.processor(),
        'environment': {'removed_keys': REMOVED_ENV, 'LC_ALL': 'C'},
        'compiler': checked(['rustc', '-vV']),
        'versions': {name: checked([path, '--version']).strip() for name, path in executables.items()},
        'binaries': {k: identity(p) for k, p in binaries},
        'sources': [identity(p) for p in sources],
        'fixture_sha256': hashlib.sha256(fixture_path.read_bytes()).hexdigest(),
        'commands': commands,
        'settings': {'sizes': args.sizes, 'rounds': args.rounds, 'samples': 11,
                     'warmup_ms_per_case': 100, 'target_batch_ms': 10,
                     'max_batch': 10000, 'startup_warmups': 3, 'startup_samples': 15,
                     'php_flags': php_flags, 'python_flags': ['-I']},
        'runs': [], 'startup_ms': {name: [] for name in runtimes},
    }
    for round_index in range(args.rounds):
        rotation = round_index % len(runtimes)
        order = runtimes[rotation:] + runtimes[:rotation]
        for runtime in order:
            print(f'Round {round_index + 1}/{args.rounds}: {runtime}', flush=True)
            raw = checked(commands[runtime])
            result = json.loads(raw)
            if result['runtime'] != runtime:
                raise RuntimeError(f'incorrect runtime identity: {runtime}')
            (args.output / f'{round_index + 1}-{runtime}.json').write_text(raw, encoding='utf-8')
            evidence['runs'].append({'round': round_index + 1, **result})
    with tempfile.TemporaryDirectory(prefix='scripta-callback-startup-') as temp:
        empty = Path(temp) / 'empty.scripta'
        empty.write_text('', encoding='utf-8')
        startup_commands = {
            'scripta': [str(args.scripta), 'run', str(empty)],
            'node': [executables['node'], '-e', ''],
            'php': [executables['php'], *php_flags, '-r', ''],
            'python': [executables['python'], '-I', '-c', ''],
        }
        for sample in range(18):
            rotation = sample % len(runtimes)
            for runtime in runtimes[rotation:] + runtimes[:rotation]:
                start = time.perf_counter_ns()
                completed = subprocess.run(startup_commands[runtime], cwd=ROOT, check=True,
                                           capture_output=True, env=ENV, timeout=60)
                elapsed = (time.perf_counter_ns() - start) / 1e6
                if completed.stdout != b'':
                    raise RuntimeError(f'unexpected startup output: {runtime}')
                if sample >= 3:
                    evidence['startup_ms'][runtime].append(elapsed)
    evidence['finished_utc'] = datetime.now(timezone.utc).isoformat()
    evidence['summary'] = summarize(evidence['runs'], args.sizes, runtimes)
    evidence['startup_medians_ms'] = {k: statistics.median(v) for k, v in evidence['startup_ms'].items()}
    (args.output / 'evidence.json').write_text(json.dumps(evidence, indent=2), encoding='utf-8')
    print('size,operation,' + ','.join(name + '_ms' for name in runtimes))
    for row in evidence['summary']:
        numbers = ','.join(f"{row[name]['median_ms']:.6f}" if name in row else 'n/a' for name in runtimes)
        print(f"{row['size']},{row['operation']},{numbers}")
    print('Startup medians (ms):', evidence['startup_medians_ms'])


if __name__ == '__main__':
    main()
