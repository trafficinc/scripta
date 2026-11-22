#!/usr/bin/env python3
"""Preview or remove selected project-local build outputs (Python 3.9+, Git required)."""
import argparse
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CACHE_ROOTS = ('scripts', 'benchmarks', 'crates', 'playground', 'tests', 'examples')


def linked(info):
    # Windows junctions and other reparse points must not escape the project.
    return stat.S_ISLNK(info.st_mode) or bool(
        getattr(info, 'st_file_attributes', 0)
        & getattr(stat, 'FILE_ATTRIBUTE_REPARSE_POINT', 0))


def git_tracked(root):
    def git(*args):
        return subprocess.run(['git', '-C', str(root), *args], check=True,
                              capture_output=True, timeout=30).stdout
    top = Path(os.fsdecode(git('rev-parse', '--show-toplevel')).strip()).resolve()
    if top != root:
        raise ValueError('The script must belong to the Git repository root.')
    # Index entries protect staged files and tracked files deleted from disk too.
    return [Path(os.fsdecode(name)) for name in git('ls-files', '-z').split(b'\0') if name]


def candidates(root, dist=False, benchmark_results=False):
    paths = [Path('target')]
    if dist:
        paths.append(Path('dist'))
    if benchmark_results:
        paths.append(Path('benchmarks/results'))
    for name in CACHE_ROOTS:
        base = root / name
        if not base.exists() or linked(base.lstat()):
            continue
        for directory, dirs, _ in os.walk(base, followlinks=False):
            parent = Path(directory)
            keep = []
            for child in sorted(dirs):
                path = parent / child
                if linked(path.lstat()):
                    continue
                if child == '__pycache__':
                    paths.append(path.relative_to(root))
                elif not child.startswith('.') and child not in (
                        'target', 'dist', 'node_modules', 'venv', 'env', 'results', 'evidence'):
                    keep.append(child)
            dirs[:] = keep
    return sorted(set(paths))


def inspect(root, relative, tracked):
    if relative.is_absolute() or '..' in relative.parts or not relative.parts:
        raise ValueError(f'Unsafe cleanup path: {relative}')
    path = root
    for part in relative.parts:
        path = path / part
        try:
            info = path.lstat()
        except FileNotFoundError:
            return None
        if linked(info) or not stat.S_ISDIR(info.st_mode):
            raise ValueError(f'Expected an ordinary directory: {path.relative_to(root)}')
    if any(entry == relative or entry.is_relative_to(relative)
           or relative.is_relative_to(entry) for entry in tracked):
        raise ValueError(f'Refusing cleanup containing a tracked path: {relative}')
    total = 0
    pending = [path]
    while pending:
        with os.scandir(pending.pop()) as entries:
            for entry in entries:
                info = entry.stat(follow_symlinks=False)
                if entry.name == '.git' or linked(info):
                    raise ValueError(f'Refusing Git metadata or linked content inside: {relative}')
                if stat.S_ISDIR(info.st_mode):
                    pending.append(Path(entry.path))
                elif stat.S_ISREG(info.st_mode):
                    total += info.st_size
                else:
                    raise ValueError(f'Refusing special filesystem content inside: {relative}')
    return total


def human_size(size):
    for unit in ('B', 'KiB', 'MiB', 'GiB', 'TiB'):
        if size < 1024 or unit == 'TiB':
            return f'{size:.1f} {unit}'
        size /= 1024


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true', help='delete the previewed categories')
    parser.add_argument('--dist', action='store_true', help='also delete dist, including validation records')
    parser.add_argument('--benchmark-results', action='store_true',
                        help='also delete benchmarks/results (local measurement records)')
    args = parser.parse_args(argv)
    root = ROOT.resolve()
    try:
        tracked = git_tracked(root)
        paths = candidates(root, args.dist, args.benchmark_results)
        # Validate every candidate before deleting anything.
        plan = []
        print(f'Project: {root}', flush=True)
        for relative in paths:
            size = inspect(root, relative, tracked)
            if size is not None:
                plan.append((relative, size))
                print(f'  {human_size(size):>12}  {relative.as_posix()}', flush=True)
        print(f'Estimated file bytes: {human_size(sum(size for _, size in plan))}; '
              'actual disk space recovered may differ.')
        if not args.apply:
            print('Preview only. Use --apply with the same options to delete these directories.')
            return 0
        # Refresh the index and recheck each directory immediately before removal.
        tracked = git_tracked(root)
        for relative, _ in plan:
            if inspect(root, relative, tracked) is not None:
                shutil.rmtree(root / relative)
                print(f'Removed {relative.as_posix()}', flush=True)
        print('Cleanup complete. The next Cargo build will regenerate build output.')
        return 0
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f'Cleanup stopped: {error}. Earlier removals, if any, are not rolled back.',
              file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
