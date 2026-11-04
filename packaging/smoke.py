#!/usr/bin/env python3
"""Exercise an installed/staged app and CLI from an unrelated working directory."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--installed', type=Path, required=True, help='VibeCut.app on macOS; installation folder on Windows')
parser.add_argument('--media', type=Path, required=True, help='Synthetic media fixture to probe')
parser.add_argument('--signed', action='store_true', help='Verify Developer ID/Gatekeeper; hardened runtime ignores DYLD diagnostics')
parser.add_argument('--output', type=Path, required=True, help='New evidence directory')
args = parser.parse_args()
installed = args.installed.resolve()
args.output = args.output.resolve()
args.output.mkdir(parents=True, exist_ok=False)
mac = platform.system() == 'Darwin'
if not mac and platform.system() != 'Windows':
    parser.error('Only macOS and Windows are supported')
binaries = installed / 'Contents/MacOS' if mac else installed
suffix = '' if mac else '.exe'
env = {k: v for k, v in os.environ.items() if not k.startswith(('DYLD_', 'FFMPEG_', 'PKG_CONFIG_'))}
# System tools only: a developer FFmpeg on PATH cannot satisfy missing libraries.
env['PATH'] = '/usr/bin:/bin:/usr/sbin:/sbin' if mac else str(Path(os.environ['SystemRoot']) / 'System32')
if mac and not args.signed:
    env['DYLD_PRINT_LIBRARIES'] = '1'
results = []
with tempfile.TemporaryDirectory(prefix='vibecut-package-smoke-') as directory:
    project = Path(directory) / 'project with spaces.vibecut'
    cli = binaries / ('vibecut-cli' + suffix)
    commands = [['probe', str(args.media.resolve()), '--json'], ['project', 'new', str(project), '--json'],
                ['exec', str(project), 'add_track', '{"name":"Video","kind":"video"}', '--json'],
                ['project', 'info', str(project), '--json']]
    for index, command in enumerate(commands):
        result = subprocess.run([str(cli), *command], env=env, cwd=directory, capture_output=True, text=True, timeout=30)
        (args.output / f'cli-{index}.stderr').write_text(result.stderr)
        (args.output / f'cli-{index}.json').write_text(result.stdout)
        if result.returncode != 0:
            raise SystemExit(f'CLI smoke {index} failed: {result.returncode}')
        json.loads(result.stdout)
        results.append({'command': command, 'exit_code': result.returncode})
    app_command = [str(binaries / ('vibecut' + suffix)), '--observe', '8', str(args.output / 'app')]
    if mac and args.signed:
        for command in (['codesign', '--verify', '--strict', str(installed)],
                        ['spctl', '--assess', '--type', 'execute', '--verbose', str(installed)],
                        ['xcrun', 'stapler', 'validate', str(installed)]):
            subprocess.run(command, check=True, capture_output=True, text=True)
        process = subprocess.Popen(app_command, env=env, cwd=directory, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            time.sleep(3)
            loaded = subprocess.run(['/usr/sbin/lsof', '-p', str(process.pid), '-Fn'], capture_output=True, text=True, check=True)
            (args.output / 'loaded-modules.txt').write_text(loaded.stdout)
            for name in ('libavcodec.', 'libavformat.', 'libavutil.', 'libswscale.', 'libswresample.'):
                paths = [line[1:] for line in loaded.stdout.splitlines() if line.startswith('n') and name in line]
                if not paths or any(not p.startswith(str(installed / 'Contents/Frameworks') + '/') for p in paths):
                    raise SystemExit('Signed app bundled-library load audit failed')
            stdout, stderr = process.communicate(timeout=45)
            app = subprocess.CompletedProcess(app_command, process.returncode, stdout, stderr)
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
    else:
        app = subprocess.run(app_command, env=env, cwd=directory, capture_output=True, text=True, timeout=45)
    (args.output / 'app.stderr').write_text(app.stderr)
    exit_receipt = json.loads((args.output / 'app/exit.json').read_text())
    if app.returncode != 0 or not exit_receipt['completed'] or not (args.output / 'app/window.png').is_file():
        raise SystemExit('App launch/capture/completion smoke failed')
    if mac and not args.signed:
        for log in args.output.glob('*.stderr'):
            loaded = [line for line in log.read_text().splitlines()
                      if line.startswith('dyld[') and any(name in line for name in ('libavcodec.', 'libavformat.', 'libavutil.', 'libswscale.', 'libswresample.'))]
            if not loaded or any(str(installed / 'Contents/Frameworks') not in line for line in loaded):
                raise SystemExit(f'Bundled FFmpeg runtime load audit failed: {log.name}')
    elif not mac:
        # The Windows loader receipt must additionally be captured with the module command in README.
        results.append({'windows_module_paths': 'unverified; capture module paths per README'})
(args.output / 'smoke.json').write_text(json.dumps({'commands': results, 'app': exit_receipt, 'release_acceptance': False}, indent=2) + '\n')
print(args.output / 'smoke.json')
