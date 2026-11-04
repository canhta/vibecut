import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('hosted_ci', Path(__file__).with_name('hosted_ci.py'))
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)
SHA = 'a' * 40


def run(conclusion='', status='completed', sha=SHA, ident=1):
    return dict(databaseId=ident, headSha=sha, status=status,
                conclusion=conclusion, url=f'https://example.invalid/{ident}')


class HostedCiTest(unittest.TestCase):
    def test_terminal_conclusions_and_url(self):
        for conclusion in ('success', 'failure', 'cancelled', 'timed_out', 'skipped', 'neutral'):
            with self.subTest(conclusion=conclusion), patch('builtins.print') as output:
                self.assertEqual(ci.wait(SHA, query=lambda *args: [run(conclusion)]),
                                 0 if conclusion == 'success' else 1)
                self.assertIn('https://example.invalid/1', output.call_args.args[0])

    def test_exact_sha_and_newest_run(self):
        query = lambda *args: [run('success', sha='b' * 40, ident=9),
                               run('success'), run('failure', ident=2)]
        with patch('builtins.print'):
            self.assertEqual(ci.wait(SHA, query=query), 1)

    def test_missing_run_and_queued_macos_time_out(self):
        for runs in ([], [run(status='queued')], [run(status='in_progress')],
                     [run('success', sha='b' * 40)]):
            with self.subTest(runs=runs), patch('builtins.print') as output:
                ticks = iter([0, ci.QUEUE_LIMIT])
                def query(*args):
                    return runs if args[1] == 'list' else {'jobs': [
                        {'name': 'Rust (macos-latest)', 'status': 'queued'}]}
                self.assertEqual(ci.wait(SHA, query=query, now=lambda: next(ticks)), 2)
                self.assertIn('fallback: run locally', output.call_args.args[0])

    def test_running_macos_is_not_queue_time(self):
        calls = 0
        def query(*args):
            nonlocal calls
            if args[1] == 'view':
                return {'jobs': [{'name': 'Rust (macos-latest)', 'status': 'in_progress'}]}
            calls += 1
            return [run('success')] if calls == 3 else [run(status='in_progress')]
        ticks = iter([0, 0, ci.QUEUE_LIMIT + 1, ci.QUEUE_LIMIT + 2])
        with patch('builtins.print'):
            self.assertEqual(ci.wait(SHA, query=query, now=lambda: next(ticks), sleep=lambda _: None), 0)


if __name__ == '__main__':
    unittest.main()
