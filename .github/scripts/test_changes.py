import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('changes', Path(__file__).with_name('changes.py'))
changes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(changes)


class ChangesTest(unittest.TestCase):
    def test_classification(self):
        for paths, expected in [([], False), (['README.md', 'docs/brand/logo.svg'], True),
                                (['crates/media/README.md'], True), (['Cargo.lock'], False),
                                (['README.md', 'apps/app/src/main.rs'], False),
                                (['.github/workflows/ci.yml'], False)]:
            with self.subTest(paths=paths):
                self.assertEqual(changes.docs_only(paths), expected)

    def test_initial_and_manual_runs_are_full(self):
        self.assertEqual(changes.changed_paths({'before': '0' * 40}, 'push'), [])
        self.assertEqual(changes.changed_paths({}, 'workflow_dispatch'), [])

    def test_pr_signoffs_are_required(self):
        event = {'pull_request': {'base': {'sha': 'base'}, 'head': {'sha': 'head'}}}
        for trailer, passes in [('Signed-off-by: Contributor <contributor@example.invalid>', True),
                                ('signed-off-by: Contributor <contributor@example.invalid>', True),
                                ('', False), ('Signed-off-by: missing email', False)]:
            with self.subTest(trailer=trailer), patch.object(changes.subprocess, 'check_output',
                    side_effect=[b'1234\n', ('message\n\n' + trailer + '\n').encode()]):
                if passes:
                    changes.require_signoffs(event)
                else:
                    with self.assertRaises(SystemExit):
                        changes.require_signoffs(event)

    def test_ticket_classification_covers_the_entire_branch(self):
        script = Path(__file__).with_name('changes.py').resolve()
        with tempfile.TemporaryDirectory() as folder:
            def git(*args):
                return subprocess.check_output(['git', '-C', folder, *args]).decode().strip()
            git('init', '-q')
            git('config', 'user.name', 'test')
            git('config', 'user.email', 'test@example.invalid')
            Path(folder, 'README.md').write_text('before')
            git('add', '.')
            git('commit', '-qm', 'base')
            base = git('rev-parse', 'HEAD')
            git('update-ref', 'refs/remotes/origin/main', base)
            for code in (False, True):
                if code:
                    Path(folder, 'code.rs').write_text('fn main() {}')
                    git('add', '.')
                    git('commit', '-qm', 'code')
                before = git('rev-parse', 'HEAD')
                Path(folder, 'README.md').write_text(f'docs {code}')
                git('commit', '-qam', 'docs follow-up')
                after = git('rev-parse', 'HEAD')
                event = Path(folder, 'event.json')
                output = Path(folder, 'output')
                for branch, expected in [('main', True), ('canhta/ticket', not code),
                                         ('feat/nested/ticket', not code)]:
                    for previous in (before, '0' * 40):
                        expected_run = expected if previous == before or branch != 'main' else False
                        with self.subTest(branch=branch, code=code, initial=previous == '0' * 40):
                            event.write_text(json.dumps({'before': previous, 'after': after}))
                            output.write_text('')
                            subprocess.run(['python3', str(script)], cwd=folder, check=True, env={
                                **os.environ, 'GITHUB_EVENT_PATH': str(event), 'GITHUB_EVENT_NAME': 'push',
                                'GITHUB_REF': 'refs/heads/' + branch, 'GITHUB_OUTPUT': str(output)})
                            self.assertEqual(output.read_text(), f'docs-only={str(expected_run).lower()}\n')

    def test_real_diff_preserves_deleted_renamed_and_hostile_paths(self):
        with tempfile.TemporaryDirectory() as folder:
            def git(*args):
                return subprocess.check_output(['git', '-C', folder, *args]).decode().strip()
            git('init', '-q')
            git('config', 'user.name', 'test')
            git('config', 'user.email', 'test@example.invalid')
            Path(folder, 'code.rs').write_text('fn main() {}')
            Path(folder, 'removed.rs').write_text('fn removed() {}')
            git('add', '.')
            git('commit', '-qm', 'initial')
            base = git('rev-parse', 'HEAD')
            Path(folder, 'docs').mkdir()
            Path(folder, 'code.rs').rename(Path(folder, 'docs', 'code.md'))
            Path(folder, 'removed.rs').unlink()
            Path(folder, 'docs', 'hostile\nname.md').write_text('docs')
            git('add', '-A')
            git('commit', '-qm', 'move')
            head = git('rev-parse', 'HEAD')
            original = subprocess.check_output
            def output(args):
                return original(args, cwd=folder)
            with patch.object(changes.subprocess, 'check_output', side_effect=output):
                paths = changes.changed_paths({'before': base, 'after': head}, 'push')
                self.assertIn('code.rs', paths)
                self.assertIn('removed.rs', paths)
                self.assertIn('docs/hostile\nname.md', paths)
                self.assertFalse(changes.docs_only(paths))
                pr = {'pull_request': {'base': {'sha': base}, 'head': {'sha': head}}}
                self.assertEqual(set(paths), set(changes.changed_paths(pr, 'pull_request')))


if __name__ == '__main__':
    unittest.main()
