#!/usr/bin/env python3
"""Collect real Cargo executable artifacts by package."""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess

import macos

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--ffmpeg', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--debug', action='store_true', help='Local smoke only; release is the default')
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
prefix = args.ffmpeg.resolve()
args.output.mkdir(parents=True, exist_ok=False)
env = dict(os.environ, FFMPEG_DIR=str(prefix), PKG_CONFIG_PATH=str(prefix / 'lib/pkgconfig'))
if platform.system() == 'Windows':
    env['PATH'] = str(prefix / 'bin') + os.pathsep + env.get('PATH', '')
elif platform.system() == 'Darwin':
    env['MACOSX_DEPLOYMENT_TARGET'] = macos.MINIMUM_VERSION
else:
    parser.error('Only native macOS and Windows builds are supported')
command = ['cargo', 'build', '--locked', '--message-format=json-render-diagnostics', '-p', 'vibecut', '-p', 'vibecut-cli']
if not args.debug:
    command.append('--release')
invocation = ['cargo', 'xtask', 'build', '--ffmpeg', str(prefix), *command[2:]]
process = subprocess.Popen(invocation, cwd=root, env=env, stdout=subprocess.PIPE, text=True)
artifacts = {}
for line in process.stdout:
    message = json.loads(line)
    executable = message.get('executable')
    if executable:
        package = message['package_id'].split('#')[-1].split('@')[0]
        # Local path package IDs can contain only the version after '#'.
        if package not in ('vibecut', 'vibecut-cli'):
            package = Path(message['manifest_path']).parent.name
        if package in ('vibecut', 'vibecut-cli'):
            artifacts[package] = Path(executable)
if process.wait() != 0:
    raise SystemExit('Cargo build failed; no distribution inputs collected')
if set(artifacts) != {'vibecut', 'vibecut-cli'}:
    raise SystemExit(f'Missing executable artifacts: {artifacts.keys()}')
for role, path in artifacts.items():
    shutil.copy2(path, args.output / (role + ('.exe' if platform.system() == 'Windows' else '')))
(args.output / 'build.json').write_text(json.dumps({'command': command, 'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(), 'profile': 'debug' if args.debug else 'release'}, indent=2) + '\n')
