"""Release preparation gates and version edits without running builds or publishing."""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from prepare_release import Preparation, choose_version, next_version, prepare, workspace_version


class PreparationTests(unittest.TestCase):
    def fixture(self, root):
        (root / 'rust-toolchain.toml').write_text('[toolchain]\nchannel = "1.99.1"\n')
        (root / 'Cargo.toml').write_text(
            '[workspace.package]\nversion = "0.1.0"\nedition = "2024"\n'
            '[workspace.dependencies]\nexternal = "0.1.0"\n')
        (root / 'Cargo.lock').write_text(
            'version = 4\n\n[[package]]\nname = "scr-cli"\nversion = "0.1.0"\n'
            'dependencies = [\n "external",\n]\n\n[[package]]\nname = "external"\n'
            'version = "0.1.0"\nsource = "registry+https://example.invalid"\nchecksum = "abc"\n')
        snapshots = {root / name: (root / name).read_bytes() for name in ('Cargo.toml', 'Cargo.lock')}
        preparation = Preparation(root)
        preparation.cargo = Mock(return_value=Mock(stdout=json.dumps({
            'workspace_members': ['cli-id'], 'packages': [{'id': 'cli-id', 'name': 'scr-cli'}]})))
        preparation.run = Mock(return_value=Mock(returncode=1))
        return preparation, snapshots

    def test_semver_choices_and_invalid_input(self):
        for choice, expected in [('patch', '0.1.1'), ('minor', '0.2.0'), ('major', '1.0.0')]:
            self.assertEqual(next_version('0.1.0', choice), expected)
        self.assertEqual(next_version('2.9.9', 'minor'), '2.10.0')
        for version in ['0.1.0-rc.1', '01.1.0', 'v0.1.0', '0.1']:
            with self.assertRaises(ValueError):
                next_version(version, 'patch')
        with patch('builtins.input', side_effect=['invalid', 'MINOR']):
            self.assertEqual(choose_version('0.1.0'), '0.2.0')
        for answer in ['', 'cancel', EOFError()]:
            with patch('builtins.input', side_effect=[answer]):
                self.assertIsNone(choose_version('0.1.0'))

    def test_failed_checks_never_prompt_or_change_versions(self):
        with tempfile.TemporaryDirectory() as directory:
            preparation, snapshots = self.fixture(Path(directory))
            preparation.checks = Mock(side_effect=subprocess.CalledProcessError(1, ['cargo', 'test']))
            with patch('prepare_release.choose_version') as prompt:
                with self.assertRaises(subprocess.CalledProcessError):
                    prepare(preparation)
                prompt.assert_not_called()
            self.assertEqual({path: path.read_bytes() for path in snapshots}, snapshots)

    def test_cancel_after_checks_does_not_bump(self):
        with tempfile.TemporaryDirectory() as directory:
            preparation, snapshots = self.fixture(Path(directory))
            preparation.checks = Mock()
            with patch('prepare_release.choose_version', return_value=None):
                prepare(preparation)
            preparation.checks.assert_called_once()
            preparation.run.assert_not_called()
            self.assertEqual({path: path.read_bytes() for path in snapshots}, snapshots)

    def test_selected_bump_follows_successful_checks(self):
        with tempfile.TemporaryDirectory() as directory:
            preparation, snapshots = self.fixture(Path(directory))
            events = []
            preparation.checks = Mock(side_effect=lambda: events.append('checks'))
            original_bump = preparation.bump
            def bump(*arguments):
                events.append('bump')
                return original_bump(*arguments)
            preparation.bump = bump
            def choose(version):
                events.append('prompt')
                return '0.1.1'
            with patch('prepare_release.choose_version', side_effect=choose):
                prepare(preparation)
            self.assertEqual(events, ['checks', 'prompt', 'bump'])
            self.assertEqual(workspace_version((preparation.root / 'Cargo.toml').read_text()), '0.1.1')
            for call in preparation.run.call_args_list:
                self.assertNotIn(call.args[0][:2], [['git', 'tag'], ['git', 'push'], ['git', 'commit']])

    def test_existing_tag_or_concurrent_version_edit_stops_before_writes(self):
        for conflict in ['tag', 'edit']:
            with self.subTest(conflict=conflict), tempfile.TemporaryDirectory() as directory:
                preparation, snapshots = self.fixture(Path(directory))
                if conflict == 'tag':
                    preparation.run.return_value.returncode = 0
                else:
                    (preparation.root / 'Cargo.toml').write_bytes(
                        snapshots[preparation.root / 'Cargo.toml'] + b'# concurrent edit\n')
                before = {path: path.read_bytes() for path in snapshots}
                with self.assertRaises(ValueError):
                    preparation.bump(snapshots, '0.1.0', '0.1.1')
                preparation.cargo.assert_not_called()
                self.assertEqual({path: path.read_bytes() for path in snapshots}, before)

    def test_bump_preserves_dependency_pins_and_requires_locked_resolve(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            preparation, snapshots = self.fixture(root)
            self.assertEqual(preparation.bump(snapshots, '0.1.0', '0.2.0'), 'v0.2.0')
            self.assertEqual(workspace_version((root / 'Cargo.toml').read_text()), '0.2.0')
            self.assertIn('external = "0.1.0"', (root / 'Cargo.toml').read_text())
            self.assertEqual((root / 'Cargo.lock').read_bytes(), snapshots[root / 'Cargo.lock'].replace(
                b'name = "scr-cli"\nversion = "0.1.0"', b'name = "scr-cli"\nversion = "0.2.0"'))
            preparation.cargo.assert_any_call(
                'metadata', '--format-version', '1', '--locked', capture=True)

    def test_failed_or_interrupted_lock_validation_restores_original_bytes(self):
        for failure in [subprocess.CalledProcessError(1, ['cargo', 'metadata']), KeyboardInterrupt()]:
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                preparation, snapshots = self.fixture(Path(directory))
                preparation.cargo.side_effect = [preparation.cargo.return_value, failure]
                with self.assertRaises(type(failure)):
                    preparation.bump(snapshots, '0.1.0', '0.1.1')
                self.assertEqual({path: path.read_bytes() for path in snapshots}, snapshots)

    def test_checks_use_native_target_pin_toolchain_and_stop_on_first_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            preparation, _ = self.fixture(Path(directory))
            preparation.run = Mock(return_value=Mock(stdout='host: aarch64-apple-darwin\n'))
            preparation.checks()
            self.assertEqual(preparation.environment['RUSTUP_TOOLCHAIN'], '1.99.1')
            self.assertEqual(preparation.environment['MACOSX_DEPLOYMENT_TARGET'], '15.0')
            for call in preparation.cargo.call_args_list:
                self.assertIn('--target', call.args)
                self.assertIn('aarch64-apple-darwin', call.args)
            preparation.run.assert_any_call(['cargo', 'clippy', '--workspace', '--all-targets',
                '--all-features', '--locked', '--target', 'aarch64-apple-darwin', '--', '-D', 'warnings'])
            preparation.cargo.reset_mock()
            preparation.cargo.side_effect = subprocess.CalledProcessError(1, ['cargo', 'test'])
            with self.assertRaises(subprocess.CalledProcessError):
                preparation.checks()
            self.assertEqual(preparation.cargo.call_count, 1)


if __name__ == '__main__':
    unittest.main()
