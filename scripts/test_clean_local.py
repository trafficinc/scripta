"""Cleanup boundaries using disposable repositories; never clean the real workspace."""
from contextlib import redirect_stderr, redirect_stdout
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import clean_local


class CleanupTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name).resolve()
        self.git('init', '-q')

    def git(self, *args):
        return subprocess.run(['git', '-C', str(self.root), *args], check=True,
                              capture_output=True)

    def write(self, name, content=b'fixture\n'):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return path

    def run_cleanup(self, *args):
        output = io.StringIO()
        with patch.object(clean_local, 'ROOT', self.root), redirect_stdout(output), redirect_stderr(output):
            status = clean_local.main(list(args))
        return status, output.getvalue()

    def test_preview_and_apply_preserve_project_and_optional_records(self):
        for newline in (b'\n', b'\r\n'):
            with self.subTest(newline=newline):
                source = self.write('crates/example/src/lib.rs', b'// source' + newline)
                manifest = self.write('Cargo.toml', b'[workspace]' + newline)
                self.git('add', 'crates', 'Cargo.toml')
                preserved = [source, manifest]
                for name in ('Cargo.lock', 'rust-toolchain.toml', 'docs/notes.md',
                             'examples/new.scripta', '.env', '.scripta/packages/local.txt',
                             'dist/validation.json', 'benchmarks/results/local.json',
                             'benchmarks/evidence/receipt.json', 'benchmarks/comparison/work.py'):
                    preserved.append(self.write(name))
                snapshots = {path: path.read_bytes() for path in preserved}
                artifact = self.write('target/debug/scripta', b'build output')
                cache = self.write('scripts/__pycache__/helper.pyc')
                status, output = self.run_cleanup()
                self.assertEqual(status, 0, output)
                self.assertIn('Preview only', output)
                self.assertTrue(artifact.exists())
                self.assertTrue(cache.exists())
                status, output = self.run_cleanup('--apply')
                self.assertEqual(status, 0, output)
                self.assertFalse(artifact.exists())
                self.assertFalse(cache.exists())
                self.assertEqual({path: path.read_bytes() for path in preserved}, snapshots)
                self.assertTrue((self.root / '.git').is_dir())

    def test_optional_outputs_require_flags_and_apply_is_repeatable(self):
        dist = self.write('dist/validation.json')
        results = self.write('benchmarks/results/local.json')
        self.assertEqual(self.run_cleanup('--dist', '--benchmark-results')[0], 0)
        self.assertTrue(dist.exists())
        self.assertTrue(results.exists())
        self.assertEqual(self.run_cleanup('--dist', '--apply')[0], 0)
        self.assertFalse(dist.exists())
        self.assertTrue(results.exists())
        for _ in range(2):
            status, output = self.run_cleanup('--benchmark-results', '--apply')
            self.assertEqual(status, 0, output)
        self.assertFalse(results.exists())

    def test_tracked_or_staged_candidates_stop_all_removal(self):
        for name in ('target/keep.txt', 'dist/keep.txt', 'scripts/__pycache__/keep.pyc',
                     'benchmarks/results/keep.json'):
            with self.subTest(name=name):
                protected = self.write(name)
                self.git('add', '-f', name)
                artifact = self.write('target/debug/output')
                status, output = self.run_cleanup('--apply', '--dist', '--benchmark-results')
                self.assertEqual(status, 1, output)
                self.assertIn('tracked', output)
                self.assertTrue(protected.exists())
                self.assertTrue(artifact.exists())
                self.git('rm', '--cached', '-f', name)

    def test_missing_tracked_file_still_protects_candidate(self):
        path = self.write('target/deleted.txt')
        self.git('add', '-f', 'target/deleted.txt')
        path.unlink()
        artifact = self.write('target/debug/output')
        status, output = self.run_cleanup('--apply')
        self.assertEqual(status, 1, output)
        self.assertTrue(artifact.exists())

    def test_symlink_candidates_and_descendants_are_rejected(self):
        # Use another disposable directory to verify the outside destination survives.
        with tempfile.TemporaryDirectory() as directory:
            outside = Path(directory)
            sentinel = outside / 'keep.txt'
            sentinel.write_bytes(b'keep')
            for name in ('target', 'target/nested'):
                with self.subTest(name=name):
                    link = self.root / name
                    link.parent.mkdir(parents=True, exist_ok=True)
                    link.symlink_to(outside, target_is_directory=True)
                    status, output = self.run_cleanup('--apply')
                    self.assertEqual(status, 1, output)
                    self.assertEqual(sentinel.read_bytes(), b'keep')
                    self.assertTrue(link.is_symlink())
                    link.unlink()

    def test_symlink_cache_parent_is_not_traversed(self):
        cache = self.write('external/__pycache__/keep.pyc')
        (self.root / 'scripts').symlink_to(self.root / 'external', target_is_directory=True)
        status, output = self.run_cleanup('--apply')
        self.assertEqual(status, 0, output)
        self.assertTrue(cache.exists())

    def test_nested_git_metadata_is_preserved(self):
        for name in ('target/vendor/.git/config', 'target/worktree/.git'):
            protected = self.write(name)
            status, output = self.run_cleanup('--apply')
            self.assertEqual(status, 1, output)
            self.assertTrue(protected.exists())

    def test_unexpected_file_at_directory_path_is_preserved(self):
        path = self.write('target')
        status, output = self.run_cleanup('--apply')
        self.assertEqual(status, 1, output)
        self.assertEqual(path.read_bytes(), b'fixture\n')

    def test_git_failure_prevents_removal(self):
        artifact = self.write('target/debug/output')
        with patch.object(clean_local, 'git_tracked', side_effect=FileNotFoundError('git unavailable')):
            status, output = self.run_cleanup('--apply')
        self.assertEqual(status, 1, output)
        self.assertTrue(artifact.exists())

    def test_repository_root_is_required(self):
        child = self.root / 'child'
        child.mkdir()
        with self.assertRaisesRegex(ValueError, 'repository root'):
            clean_local.git_tracked(child)

    def test_escaping_paths_are_rejected(self):
        for path in (Path('..'), self.root, Path('.')):
            with self.subTest(path=path), self.assertRaises(ValueError):
                clean_local.inspect(self.root, path, [])

    def test_deletion_failure_is_reported(self):
        artifact = self.write('target/debug/output')
        with patch.object(clean_local.shutil, 'rmtree', side_effect=PermissionError('denied')):
            status, output = self.run_cleanup('--apply')
        self.assertEqual(status, 1, output)
        self.assertIn('Cleanup stopped', output)
        self.assertTrue(artifact.exists())


if __name__ == '__main__':
    unittest.main()
