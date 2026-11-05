#!/usr/bin/env python3
"""Export native inputs or combine matched native inputs before signing."""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess

from package import compliance, mac_dependencies, mac_relocate
import macos


def run(*args):
    return subprocess.check_output([str(arg) for arg in args], text=True)


def export(prefix, binaries, output):
    output.mkdir(parents=True, exist_ok=False)
    shutil.copytree(binaries, output / 'binaries')
    sdk = output / 'sdk/prefix'
    (sdk / 'lib').mkdir(parents=True)
    # Validate and retain corresponding source material before cross-job transfer.
    compliance(prefix, output / 'verified-compliance')
    shutil.copy2(prefix / 'BUILD.txt', sdk)
    shutil.copytree(prefix / 'compliance', sdk / 'compliance')
    shutil.copytree(output / 'verified-compliance/sources', sdk.parent / 'sources')
    shutil.rmtree(output / 'verified-compliance')
    for path in (prefix / 'lib').glob('*.dylib'):
        if not path.is_symlink():
            shutil.copy2(path, sdk / 'lib' / Path(mac_dependencies(path)[0]).name)
    libs = sorted((sdk / 'lib').glob('*.dylib'))
    mac_relocate(libs + [output / 'binaries/vibecut', output / 'binaries/vibecut-cli'], libs)


def combine(arm, intel, output):
    revisions = [json.loads((root / 'binaries/build.json').read_text()) for root in (arm, intel)]
    if revisions[0] != revisions[1]:
        raise ValueError('Native executable build revisions/profiles differ')
    receipts = [(root / 'sdk/prefix/BUILD.txt').read_text() for root in (arm, intel)]
    identities = [re.findall(r'^(ffmpeg|opus|libvpx): ([\d.]+)\nsource: (\S+)\nsha256: ([a-f0-9]{64})$', text, re.M) for text in receipts]
    if len(identities[0]) != 3 or identities[0] != identities[1]:
        raise ValueError('Native media versions/source identities differ')
    libraries = [set(p.name for p in (root / 'sdk/prefix/lib').glob('*.dylib')) for root in (arm, intel)]
    if len(libraries[0]) < 6 or libraries[0] != libraries[1]:
        raise ValueError('Native FFmpeg dylib sets differ')
    # Compliance checks both SDKs, including their source/patch hashes.
    output.mkdir(parents=True, exist_ok=False)
    for root, arch in ((arm, 'aarch64'), (intel, 'x86_64')):
        compliance(root / 'sdk/prefix', output / ('verify-' + arch))
        shutil.rmtree(output / ('verify-' + arch))
    shutil.copytree(arm / 'sdk', output / 'sdk')
    shutil.copytree(arm / 'binaries', output / 'binaries')
    provenance = output / 'sdk/prefix/compliance/architectures'
    provenance.mkdir()
    for root, arch in ((arm, 'aarch64'), (intel, 'x86_64')):
        shutil.copy2(root / 'sdk/prefix/BUILD.txt', provenance / (arch + '-BUILD.txt'))
    for relative in [Path('binaries/vibecut'), Path('binaries/vibecut-cli')] + [Path('sdk/prefix/lib') / name for name in sorted(libraries[0])]:
        macos.audit(arm / relative, 'aarch64', run)
        macos.audit(intel / relative, 'x86_64', run)
        target = output / relative
        target.unlink()
        run('lipo', '-create', arm / relative, intel / relative, '-output', target)
        target.chmod(0o755)
        macos.audit(target, 'universal', run)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    native = commands.add_parser('export')
    native.add_argument('prefix', type=Path)
    native.add_argument('binaries', type=Path)
    native.add_argument('output', type=Path)
    fat = commands.add_parser('combine')
    fat.add_argument('arm', type=Path)
    fat.add_argument('intel', type=Path)
    fat.add_argument('output', type=Path)
    args = vars(parser.parse_args())
    command = args.pop('command')
    (export if command == 'export' else combine)(**args)
