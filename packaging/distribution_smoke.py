#!/usr/bin/env python3
"""Clean-runner smoke; no media tools, GUI, or external library paths required."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile


def smoke(installed, output):
    installed, output = installed.resolve(), output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    mac = platform.system() == 'Darwin'
    binaries = installed / 'Contents/MacOS' if mac else installed
    suffix = '' if mac else '.exe'
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(('DYLD_', 'FFMPEG_', 'PKG_CONFIG_'))}
    env['PATH'] = '/usr/bin:/bin:/usr/sbin:/sbin' if mac else str(Path(os.environ['SystemRoot']) / 'System32')
    receipts = []
    with tempfile.TemporaryDirectory(prefix='vibecut-distribution-') as temporary:
        project = Path(temporary) / 'synthetic.vibecut'
        cli = binaries / ('vibecut-cli' + suffix)
        def invoke(*args):
            result = subprocess.run([str(cli), *map(str, args), '--json'], env=env,
                                    cwd=temporary, capture_output=True, text=True, timeout=180)
            receipts.append({'args': list(map(str, args)), 'exit': result.returncode,
                             'stdout': result.stdout, 'stderr': result.stderr})
            if result.returncode:
                raise RuntimeError(f'Distribution CLI smoke failed: {args[0]}: {result.stderr}')
            return json.loads(result.stdout)
        try:
            invoke('--version')
            invoke('project', 'new', project)
            # The project file is produced by its owner; the existing registry owns edits.
            # Reduce geometry for a bounded two-second packaging check.
            data = json.loads(project.read_text())
            data['project']['canvas']['width'] = 64
            data['project']['canvas']['height'] = 64
            project.write_text(json.dumps(data))
            invoke('exec', project, 'track.add', '{"name":"Synthetic","kind":"video"}')
            invoke('project', 'info', project)
            # Read the saved project rather than hardcoding the allocated track id.
            data = json.loads(project.read_text())
            track = data['project']['timeline']['overlays'][0]['id']
            params = {'track': track, 'item': {'animations': {}, 'source_asset': None,
                      'source_range': {'start': 0, 'end': 508032000000}, 'start': 0,
                      'content': {'kind': 'overlay', 'params': {'color': [30, 160, 90, 255]}}}}
            invoke('exec', project, 'insert_item', json.dumps(params))
            exported = output / 'synthetic.mp4'
            invoke('export', project, '--out', exported)
            probe = invoke('probe', exported)
            # Persist the probe so timing/stream admission can be inspected without oracles.
            (output / 'export-probe.json').write_text(json.dumps(probe, indent=2))
            if exported.stat().st_size == 0 or abs(probe['duration']['seconds'] - 2.0) > 0.1:
                raise RuntimeError('Synthetic export must contain two seconds')
            video = [stream for stream in probe['streams'] if stream['kind'] == 'video']
            if len(video) != 1:
                raise RuntimeError('Synthetic export needs one video stream')
            app = subprocess.run([str(binaries / ('vibecut' + suffix)), '--headless-smoke',
                                  str(Path(temporary) / 'workspace')], env=env, cwd=temporary,
                                 capture_output=True, text=True, timeout=60)
            receipts.append({'app': 'headless workspace/frontend startup', 'exit': app.returncode,
                             'stdout': app.stdout, 'stderr': app.stderr})
            if app.returncode:
                raise RuntimeError(f'Distribution app smoke failed: {app.stderr}')
        finally:
            (output / 'smoke.json').write_text(json.dumps(receipts, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--installed', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    smoke(**vars(parser.parse_args()))
