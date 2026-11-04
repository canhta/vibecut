#!/usr/bin/env python3
"""Wait for push CI on an exact commit; exit 2 permits the queued fallback."""
import json
import subprocess
import sys
import time

QUEUE_LIMIT = 45 * 60
POLL = 20


def gh(*args):
    return json.loads(subprocess.check_output(['gh', *args], timeout=60))


def wait(sha):
    queued = 0
    previous = time.monotonic()
    waiting = True
    url = '(run not registered yet)'
    while True:
        runs = gh('run', 'list', '--workflow', 'ci.yml', '--event', 'push',
                     '--commit', sha, '--limit', '100', '--json',
                     'databaseId,headSha,status,conclusion,url')
        # Never accept a prefix match or an older attempt when a newer run exists.
        runs = [run for run in runs if run['headSha'] == sha]
        run = max(runs, key=lambda run: run['databaseId']) if runs else None
        current = time.monotonic()
        if waiting:
            queued += current - previous
        previous = current
        if run:
            url = run['url']
            if run['status'] == 'completed':
                passed = run['conclusion'] == 'success'
                print(f"{'pass' if passed else 'fail'}: {sha} ({run['conclusion']}) {url}")
                return 0 if passed else 1
            jobs = gh('run', 'view', str(run['databaseId']), '--json', 'jobs')['jobs']
            # Workflow status can be in_progress while the macOS runner is queued.
            waiting = not any(job['name'] == 'Rust (macos-latest)' and
                              job['status'] in ('in_progress', 'completed') for job in jobs)
        else:
            waiting = True
        if waiting and queued >= QUEUE_LIMIT:
            print(f'fail: {sha} queued for 45 min {url}; fallback: run locally')
            return 2
        time.sleep(POLL)


def main():
    if len(sys.argv) != 2:
        raise SystemExit('usage: hosted-ci <sha>')
    try:
        sha = subprocess.check_output([
            'git', 'rev-parse', '--verify', '--end-of-options', sys.argv[1] + '^{commit}'
        ], text=True, timeout=10).strip()
        return wait(sha)
    except (subprocess.SubprocessError, OSError, ValueError, KeyError) as error:
        print(f'fail: cannot inspect hosted CI: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
