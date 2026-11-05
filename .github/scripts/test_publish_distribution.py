import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('publishing', Path(__file__).with_name('publish_distribution.py'))
publishing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publishing)


class PublishDistributionTest(unittest.TestCase):
    def distributions(self, directory, extension='pkg'):
        for arch in ('aarch64', 'x86_64', 'universal'):
            (directory / f'VibeCut-0.1.0-macos-{arch}.{extension}').write_bytes(b'synthetic')

    def test_missing_or_foreign_assets_refuse_before_any_remote_mutation(self):
        with tempfile.TemporaryDirectory() as folder, patch.object(publishing, 'api') as api:
            directory = Path(folder)
            for names in ([], ['unexpected.pkg']):
                for name in names:
                    (directory / name).write_bytes(b'synthetic')
                with self.assertRaises(ValueError):
                    publishing.publish('nightly', directory)
            api.assert_not_called()

    def test_stable_release_and_superseded_build_leave_remote_unchanged(self):
        with tempfile.TemporaryDirectory() as folder, patch.dict(os.environ, GITHUB_SHA='new'):
            directory = Path(folder)
            self.distributions(directory)
            for release, head in (({'prerelease': False}, 'new'), (None, 'later')):
                with self.subTest(release=release, head=head):
                    calls = []
                    def api(method, path, *args, **kwargs):
                        calls.append((method, path))
                        if path.startswith('releases/'):
                            return release
                        if path == 'git/ref/heads/main':
                            return {'object': {'sha': head}}
                        return None
                    with patch.object(publishing, 'api', side_effect=api), patch.object(publishing, 'gh') as gh:
                        if release:
                            with self.assertRaises(ValueError):
                                publishing.publish('nightly', directory)
                        else:
                            publishing.publish('nightly', directory)
                        self.assertTrue(all(method == 'GET' for method, _ in calls))
                        gh.assert_not_called()

    def test_failed_upload_preserves_tag_notes_and_old_assets(self):
        with tempfile.TemporaryDirectory() as folder, patch.dict(os.environ, GITHUB_SHA='new', GH_REPO='owner/repo'):
            directory = Path(folder)
            self.distributions(directory)
            calls = []
            def api(method, path, *args, **kwargs):
                calls.append((method, path))
                if path.startswith('releases/'):
                    return {'id': 1, 'prerelease': True, 'assets': [{'id': 2, 'name': 'old.pkg'}]}
                if path == 'git/ref/heads/main':
                    return {'object': {'sha': 'new'}}
                if path == 'git/ref/tags/nightly':
                    return {'object': {'sha': 'old', 'type': 'commit'}}
                return {'status': 'ahead', 'commits': []}
            with patch.object(publishing, 'api', side_effect=api), patch.object(publishing, 'gh', side_effect=RuntimeError('upload failed')):
                with self.assertRaisesRegex(RuntimeError, 'upload failed'):
                    publishing.publish('nightly', directory)
            self.assertTrue(all(method == 'GET' for method, _ in calls))

    def test_nightly_moves_tag_after_upload_and_records_commits(self):
        with tempfile.TemporaryDirectory() as folder, patch.dict(os.environ, GITHUB_SHA='new', GH_REPO='owner/repo'):
            directory = Path(folder)
            self.distributions(directory)
            calls = []
            def api(method, path, data=None, **kwargs):
                calls.append((method, path, data))
                if method != 'GET':
                    return None
                if path.startswith('releases/'):
                    return {'id': 1, 'prerelease': True, 'assets': [{'id': 2, 'name': 'old.pkg'}]}
                if path == 'git/ref/heads/main':
                    return {'object': {'sha': 'new'}}
                if path == 'git/ref/tags/nightly':
                    return {'object': {'sha': 'old', 'type': 'commit'}}
                return {'status': 'ahead', 'commits': [{'sha': 'abcdef1234', 'commit': {'message': 'Implemented\nbody'}}]}
            with patch.object(publishing, 'api', side_effect=api), patch.object(publishing, 'gh', side_effect=lambda *a: calls.append(('UPLOAD', '', None))):
                publishing.publish('nightly', directory)
            upload = next(i for i, call in enumerate(calls) if call[0] == 'UPLOAD')
            move = next(i for i, call in enumerate(calls) if call[1] == 'git/refs/tags/nightly')
            self.assertLess(upload, move)
            self.assertIn('- abcdef1 Implemented', calls[-1][2]['body'])
            self.assertIn('SHA256SUMS', [path.name for path in publishing.assets(directory, 'nightly')])

    def test_signed_release_refuses_published_release_or_mismatched_tag(self):
        with tempfile.TemporaryDirectory() as folder, patch.dict(os.environ, GITHUB_SHA='new'):
            directory = Path(folder)
            self.distributions(directory, 'dmg')
            for draft, tag in ((False, 'new'), (True, 'old')):
                with self.subTest(draft=draft, tag=tag):
                    calls = []
                    def api(method, path, *args, **kwargs):
                        calls.append((method, path))
                        if path.startswith('releases/'):
                            return {'id': 1, 'draft': draft, 'assets': []}
                        return {'object': {'sha': tag, 'type': 'commit'}}
                    with patch.object(publishing, 'api', side_effect=api), patch.object(publishing, 'gh') as gh:
                        with self.assertRaises(ValueError):
                            publishing.publish('release', directory, '0.1.0')
                        self.assertTrue(all(method == 'GET' for method, _ in calls))
                        gh.assert_not_called()


if __name__ == '__main__':
    unittest.main()
