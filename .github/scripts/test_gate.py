import json
import os
from pathlib import Path
import subprocess
import unittest


class GateTest(unittest.TestCase):
    def test_selected_checks_fail_closed(self):
        for docs_only in (True, False):
            needs = {name: {'result': 'success'} for name in
                     ('changes', 'docs', 'lint', 'test-build', 'test')}
            needs['changes']['outputs'] = {'docs-only': str(docs_only).lower()}
            if docs_only:
                for name in ('lint', 'test-build', 'test'):
                    needs[name]['result'] = 'skipped'
            cases = [('green', needs, True)]
            for name in ('changes', 'docs') if docs_only else tuple(needs):
                for result in ('failure', 'cancelled', 'skipped', None):
                    value = json.loads(json.dumps(needs))
                    if result is None:
                        del value[name]
                    else:
                        value[name]['result'] = result
                    cases.append((f'{name}/{result}', value, False))
            for name, value, success in cases:
                with self.subTest(docs_only=docs_only, name=name):
                    result = subprocess.run(
                        ['python3', str(Path(__file__).with_name('gate.py'))],
                        env={**os.environ, 'NEEDS_JSON': json.dumps(value)}, capture_output=True)
                    self.assertEqual(result.returncode == 0, success, result.stderr.decode())
