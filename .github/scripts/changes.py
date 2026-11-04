"""Classify the complete event diff, including deleted and renamed paths."""
import json
import os
import re
import subprocess


def docs_only(paths):
    return bool(paths) and all(
        path.endswith('.md') or path.startswith('docs/') for path in paths
    )


def changed_paths(event, name):
    if name == 'pull_request':
        base = event['pull_request']['base']['sha']
        head = event['pull_request']['head']['sha']
        revision = f'{base}...{head}'
    elif name == 'push' and event.get('before', '').strip('0'):
        revision = f"{event['before']}..{event['after']}"
    else:
        return []  # Manual/initial runs always exercise the full gate.
    # --no-renames includes both sides of moves; a Rust file moved into docs
    # must still run the Rust gate. NUL separation handles hostile file names.
    result = subprocess.check_output([
        'git', 'diff', '--name-only', '--no-renames', '-z', revision, '--'
    ])
    return result.decode('utf-8', errors='surrogateescape').rstrip('\0').split('\0') if result else []


def require_signoffs(event):
    base = event['pull_request']['base']['sha']
    head = event['pull_request']['head']['sha']
    commits = subprocess.check_output(['git', 'rev-list', f'{base}..{head}']).decode().splitlines()
    for commit in commits:
        message = subprocess.check_output(['git', 'show', '-s', '--format=%B', commit]).decode()
        if not re.search(r'^Signed-off-by: .+ <[^<>\s]+@[^<>\s]+>$', message, re.M | re.I):
            raise SystemExit(f'Commit {commit} needs a Signed-off-by trailer (git commit -s)')


if __name__ == '__main__':
    with open(os.environ['GITHUB_EVENT_PATH']) as stream:
        event = json.load(stream)
    if os.environ['GITHUB_EVENT_NAME'] == 'pull_request':
        require_signoffs(event)
    # Ticket pushes always exercise the full required-oracle gate, even for docs.
    ticket = (os.environ['GITHUB_EVENT_NAME'] == 'push' and
              os.environ.get('GITHUB_REF', '').startswith(('refs/heads/canhta/', 'refs/heads/feat/')))
    value = str(not ticket and docs_only(changed_paths(event, os.environ['GITHUB_EVENT_NAME']))).lower()
    with open(os.environ['GITHUB_OUTPUT'], 'a') as stream:
        stream.write(f'docs-only={value}\n')
