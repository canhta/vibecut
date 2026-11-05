"""Fail closed on missing, failed, cancelled or unexpectedly skipped CI jobs."""
import json
import os


def main():
    needs = json.loads(os.environ['NEEDS_JSON'])
    required = ['changes', 'docs']
    if needs.get('changes', {}).get('outputs', {}).get('docs-only') != 'true':
        required += ['lint', 'test-build', 'test']
    failures = [name for name in required if needs.get(name, {}).get('result') != 'success']
    if failures:
        raise SystemExit('Required checks did not succeed: ' + ', '.join(failures))


if __name__ == '__main__':
    main()
