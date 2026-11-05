import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('sign_macos', Path(__file__).with_name('sign_macos.py'))
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)


class SigningTest(unittest.TestCase):
    def test_missing_secrets_name_fields_without_values(self):
        with self.assertRaises(ValueError) as error:
            signing.validate({'APPLE_API_KEY_P8': 'private synthetic key'})
        message = str(error.exception)
        self.assertIn('MACOS_CERT_P12', message)
        self.assertIn('MACOS_CERT_PASSWORD', message)
        self.assertNotIn('private synthetic key', message)

    def test_tool_failure_output_is_suppressed(self):
        result = subprocess.CompletedProcess([], 1, 'sensitive identity', 'sensitive password')
        with patch.object(signing.subprocess, 'run', return_value=result):
            with self.assertRaises(RuntimeError) as error:
                signing.run('security', 'import', 'private certificate')
        self.assertNotIn('sensitive', str(error.exception))
        self.assertNotIn('private certificate', str(error.exception))

    def test_invalid_certificate_cleans_private_directory(self):
        with tempfile.TemporaryDirectory() as folder:
            env = {name: 'synthetic' for name in signing.REQUIRED}
            env.update(RUNNER_TEMP=folder, MACOS_CERT_P12='invalid base64!')
            with patch.dict(os.environ, env):
                with self.assertRaises(ValueError):
                    signing.sign('prefix', 'binaries', 'output', 'aarch64')
                self.assertFalse((Path(folder) / 'vibecut-signing').exists())

    def test_cleanup_restores_search_list_deletes_keychain_and_keys(self):
        with tempfile.TemporaryDirectory() as folder, patch.dict(os.environ, RUNNER_TEMP=folder):
            directory = signing.state_dir()
            directory.mkdir()
            (directory / 'search-list.json').write_text('["/previous/login.keychain-db"]')
            (directory / 'release.keychain-db').touch()
            (directory / 'notary.p8').write_text('synthetic key')
            with patch.object(signing, 'run') as run:
                signing.cleanup()
                self.assertEqual(run.call_args_list[0].args,
                                 ('security', 'list-keychains', '-d', 'user', '-s', '/previous/login.keychain-db'))
                self.assertEqual(run.call_args_list[1].args[0:2], ('security', 'delete-keychain'))
            self.assertFalse(directory.exists())
            signing.cleanup()  # Unconditional job cleanup is idempotent.

    def test_signing_failure_suppresses_output_and_always_cleans(self):
        with tempfile.TemporaryDirectory() as folder:
            env = {name: 'synthetic' for name in signing.REQUIRED}
            env.update(RUNNER_TEMP=folder, MACOS_CERT_P12='Y2VydA==', APPLE_TEAM_ID='TESTTEAM00')
            def native(*args):
                if args[1] == 'list-keychains' and len(args) == 4:
                    return '"/login.keychain-db"'
                if args[1] == 'find-identity':
                    return '1) 012345 "Developer ID Application: Synthetic (TESTTEAM00)"'
                return ''
            with patch.dict(os.environ, env), patch.object(signing, 'run', side_effect=native), \
                    patch.object(signing.subprocess, 'run', return_value=subprocess.CompletedProcess(
                        [], 1, 'private synthetic output', 'private synthetic error')):
                with self.assertRaisesRegex(RuntimeError, '^Signed packaging/notarization failed;') as error:
                    signing.sign('prefix', 'binaries', 'output', 'aarch64')
                self.assertNotIn('private synthetic', str(error.exception))
                self.assertFalse(signing.state_dir().exists())

    def test_cleanup_still_deletes_keychain_when_search_restore_fails(self):
        with tempfile.TemporaryDirectory() as folder, patch.dict(os.environ, RUNNER_TEMP=folder):
            directory = signing.state_dir()
            directory.mkdir()
            (directory / 'search-list.json').write_text('[]')
            (directory / 'release.keychain-db').touch()
            with patch.object(signing, 'run', side_effect=[RuntimeError('security failed'), '']) as run:
                with self.assertRaises(RuntimeError):
                    signing.cleanup()
                self.assertEqual(run.call_args_list[1].args[0:2], ('security', 'delete-keychain'))
            self.assertFalse(directory.exists())


if __name__ == '__main__':
    unittest.main()
