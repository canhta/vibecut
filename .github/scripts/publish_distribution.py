"""Release boundary: validate assets before touching tags or release state."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tomllib
import subprocess
from urllib.error import HTTPError
from urllib.request import Request, urlopen


def api(method, endpoint, data=None, optional=False):
    request = Request('https://api.github.com/repos/' + os.environ['GH_REPO'] + '/' + endpoint,
                      data=json.dumps(data).encode() if data is not None else None, method=method,
                      headers={'Authorization': 'Bearer ' + os.environ['GH_TOKEN'],
                               'Accept': 'application/vnd.github+json', 'Content-Type': 'application/json',
                               'X-GitHub-Api-Version': '2026-03-10'})
    try:
        with urlopen(request, timeout=60) as response:
            content = response.read()
            return json.loads(content) if content else None
    except HTTPError as error:
        if optional and error.code == 404:
            return None
        raise RuntimeError(f'GitHub {method} {endpoint} failed: HTTP {error.code}') from None


def gh(*args):
    subprocess.run(['gh', *map(str, args)], check=True)


def assets(directory, mode):
    suffix = 'dmg' if mode == 'release' else 'pkg'
    packages = sorted(p for p in directory.iterdir() if p.suffix in ('.pkg', '.zip', '.dmg'))
    version = tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version']
    required = {f'VibeCut-{version}-macos-{arch}.{suffix}'
                for arch in ('aarch64', 'x86_64', 'universal')}
    allowed = required | ({f'VibeCut-{version}-windows-{arch}.zip'
                           for arch in ('aarch64', 'x86_64')} if mode == 'nightly' else set())
    for path in packages:
        if path.name not in allowed or path.is_symlink() or path.stat().st_size == 0:
            raise ValueError('Invalid distribution asset: ' + path.name)
    if not required.issubset({path.name for path in packages}):
        raise ValueError('All three macOS distributions must pass before publication')
    sums = directory / 'SHA256SUMS'
    lines = []
    for path in packages:
        with path.open('rb') as stream:
            lines.append(f'{hashlib.file_digest(stream, "sha256").hexdigest()}  {path.name}\n')
    sums.write_text(''.join(lines))
    return packages + [sums]


def publish(mode, directory, version=None):
    if mode == 'release' and version != tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version']:
        raise ValueError('Release version must match the workspace version')
    files = assets(directory, mode)
    sha = os.environ['GITHUB_SHA']
    tag = 'nightly' if mode == 'nightly' else 'v' + version
    existing = api('GET', 'releases/tags/' + tag, optional=True)
    reference = api('GET', 'git/ref/tags/' + tag, optional=True)
    if existing and (existing.get('immutable') or
                     (mode == 'nightly' and not existing['prerelease']) or
                     (mode == 'release' and not existing['draft'])):
        raise ValueError('Refusing to modify a stable, published or immutable release')
    if mode == 'nightly':
        main = api('GET', 'git/ref/heads/main')['object']['sha']
        if main != sha:
            print('Superseded build; leaving nightly unchanged')
            return
        previous = reference['object']['sha'] if reference else None
        if previous:
            # Only a lightweight nightly tag written by this workflow is admitted.
            if reference['object']['type'] != 'commit':
                raise ValueError('Nightly tag must refer directly to a commit')
            comparison = api('GET', f'compare/{previous}...{sha}?per_page=100&page=1')
            if comparison['status'] not in ('ahead', 'identical'):
                raise ValueError('Nightly history diverged; maintainer reconciliation required')
            commits = comparison['commits']
            total = comparison.get('total_commits', len(commits))
            page = 2
            while len(commits) < total:
                batch = api('GET', f'compare/{previous}...{sha}?per_page=100&page={page}')['commits']
                if not batch:
                    raise ValueError('Incomplete nightly commit list')
                commits.extend(batch)
                page += 1
        else:
            commits = [api('GET', 'commits/' + sha)]
        notes = 'Unsigned test builds of ' + sha + '.\n\n'
        notes += '\n'.join('- ' + commit['sha'][:7] + ' ' + commit['commit']['message'].splitlines()[0] for commit in commits)
        if previous:
            notes += f'\n\nFull commit list: https://github.com/{os.environ["GH_REPO"]}/compare/{previous}...{sha}\n'
        notes += '\nWindows targets are advisory until p0-core/44. These builds are not notarized.\n'
    else:
        if reference and reference['object']['sha'] != sha:
            raise ValueError('Existing version tag does not match the verified commit')
        notes = 'Signed and notarized macOS arm64, x86_64 and universal DMGs; clean-runner app/CLI smoke passed for both architectures.\n'
    if not existing:
        existing = api('POST', 'releases', {'tag_name': tag, 'target_commitish': sha,
                        'name': 'Nightly' if mode == 'nightly' else 'VibeCut ' + tag,
                        'draft': True, 'prerelease': mode == 'nightly', 'body': notes})
    release_id = existing['id']
    # Never clobber signed release assets; a rerun must be reconciled explicitly.
    gh('release', 'upload', tag, *files, *(['--clobber'] if mode == 'nightly' else []))
    if mode == 'release' and not api('GET', 'git/ref/tags/' + tag, optional=True):
        api('POST', 'git/refs', {'ref': 'refs/tags/' + tag, 'sha': sha})
    if mode == 'nightly':
        if api('GET', 'git/ref/heads/main')['object']['sha'] != sha:
            raise RuntimeError('Build superseded during upload; refusing to move nightly tag')
        desired = {p.name for p in files}
        # Drop obsolete architectures/versioned assets only after all new uploads succeeded.
        for asset in existing['assets']:
            if asset['name'] not in desired:
                api('DELETE', f'releases/assets/{asset["id"]}')
        if reference:
            api('PATCH', 'git/refs/tags/nightly', {'sha': sha, 'force': True})
        # Creating the release above creates its tag if this is the first nightly.
        api('PATCH', f'releases/{release_id}', {'draft': False, 'prerelease': True,
            'name': 'Nightly ' + sha[:7], 'body': notes, 'make_latest': 'false'})


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['nightly', 'release'])
    parser.add_argument('directory', type=Path)
    parser.add_argument('--version')
    args = parser.parse_args()
    if args.mode == 'release' and not args.version:
        parser.error('release requires --version')
    publish(**vars(args))
