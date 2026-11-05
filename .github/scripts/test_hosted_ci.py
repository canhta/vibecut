import importlib.util
import json
from pathlib import Path
import subprocess
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
    def test_cli_results(self):
        cases = [(conclusion, [[run(conclusion)]], [0, 0], 0 if conclusion == 'success' else 1,
                  conclusion, 'https://example.invalid/1')
                 for conclusion in ('success', 'failure', 'cancelled', 'timed_out', 'skipped', 'neutral')]
        cases += [
            ('exact SHA and newest run', [[run('success', sha='b' * 40, ident=9),
                                          run('success'), run('failure', ident=2)]],
             [0, 0], 1, 'failure', 'https://example.invalid/2'),
            ('missing run', [[]], [0, ci.QUEUE_LIMIT], 2, 'fallback: run locally', 'not registered'),
            ('wrong SHA only', [[run('success', sha='b' * 40)]],
             [0, ci.QUEUE_LIMIT], 2, 'fallback: run locally', 'not registered'),
            ('queued workflow', [[run(status='queued')]],
             [0, ci.QUEUE_LIMIT], 2, 'fallback: run locally', 'https://example.invalid/1'),
            ('queued macOS', [[run(status='in_progress')]],
             [0, ci.QUEUE_LIMIT], 2, 'fallback: run locally', 'https://example.invalid/1'),
            ('queued shard after builder', [[run(status='in_progress')]],
             [0, ci.QUEUE_LIMIT], 2, 'fallback: run locally', 'https://example.invalid/1'),
            ('running shards', [[run(status='in_progress')], [run(status='in_progress')], [run('success')]],
             [0, 0, ci.QUEUE_LIMIT + 1, ci.QUEUE_LIMIT + 2], 0, 'pass:', 'https://example.invalid/1'),
            ('running macOS', [[run(status='in_progress')], [run(status='in_progress')], [run('success')]],
             [0, 0, ci.QUEUE_LIMIT + 1, ci.QUEUE_LIMIT + 2], 0, 'pass:', 'https://example.invalid/1'),
            ('API error', None, [0], 1, 'cannot inspect hosted CI', ''),
        ]
        for name, responses, ticks, expected, message, url in cases:
            with self.subTest(name=name):
                responses = iter(responses) if responses is not None else None
                def output(command, **kwargs):
                    if command[0] == 'git':
                        return SHA + '\n'
                    if responses is None:
                        raise subprocess.CalledProcessError(1, command)
                    if command[2] == 'list':
                        self.assertEqual(command[command.index('--commit') + 1], SHA)
                        return json.dumps(next(responses))
                    if name in ('queued shard after builder', 'running shards'):
                        return json.dumps({'jobs': [
                            {'name': 'build tests (macos-arm64)', 'status': 'completed'},
                            {'name': 'test (macos-arm64, 2/3)',
                             'status': 'in_progress' if name == 'running shards' else 'queued'}]})
                    return json.dumps({'jobs': [{'name': 'build tests (macos-arm64)',
                                      'status': 'in_progress' if name == 'running macOS' else 'queued'}]})
                with patch.object(ci.sys, 'argv', ['hosted-ci', SHA]), \
                     patch.object(ci.subprocess, 'check_output', side_effect=output), \
                     patch.object(ci.time, 'monotonic', side_effect=ticks), \
                     patch.object(ci.time, 'sleep'), patch('builtins.print') as printed:
                    self.assertEqual(ci.main(), expected)
                    self.assertIn(message, printed.call_args.args[0])
                    self.assertIn(url, printed.call_args.args[0])


if __name__ == '__main__':
    unittest.main()
