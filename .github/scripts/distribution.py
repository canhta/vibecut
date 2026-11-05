"""Build/package job entry points; smoke runs on separate VMs without setup-build."""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tomllib
import zipfile


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)


def package(arch, signed, prefix, binaries):
    if signed:
        run(sys.executable, '.github/scripts/sign_macos.py', 'sign', prefix, binaries, 'target/package', arch)
    else:
        system = 'macos' if platform.system() == 'Darwin' else 'windows'
        run(sys.executable, 'packaging/package.py', '--os', system, '--arch', arch,
            '--ffmpeg', prefix, '--binaries', binaries, '--output', 'target/package')
    Path('target/dist').mkdir(parents=True, exist_ok=True)
    suffix = '*.dmg' if signed else ('*.pkg' if platform.system() == 'Darwin' else '*.zip')
    packages = list(Path('target/package').glob(suffix))
    if len(packages) != 1:
        raise ValueError('Expected one distribution package')
    copied = Path(shutil.copy2(packages[0], 'target/dist'))
    with copied.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    copied.with_name(copied.name + '.sha256').write_text(f'{digest}  {copied.name}\n')


def native(args):
    expected = {'aarch64': ('arm64', 'aarch64'), 'x86_64': ('x86_64', 'amd64')}[args.arch]
    if platform.machine().lower() not in expected:
        raise ValueError('Native runner architecture does not match the distribution')
    system = 'macos' if platform.system() == 'Darwin' else 'windows'
    home = Path(os.environ['USERPROFILE']) if system == 'windows' else Path.home()
    prefix = Path((home / '.cache/vibecut/ffmpeg' / f'{system}-{args.arch}' / 'current').read_text().strip())
    run(sys.executable, 'packaging/build.py', '--ffmpeg', prefix, '--output', 'target/inputs')
    if system == 'macos':
        run(sys.executable, 'packaging/universal.py', 'export', prefix, 'target/inputs', 'target/native')
        with tarfile.open('target/native.tar.gz', 'w:gz') as archive:
            archive.add('target/native', arcname='native')
    package(args.arch, args.signed, prefix, 'target/inputs')


def universal(args):
    for arch in ('aarch64', 'x86_64'):
        with tarfile.open(f'target/raw/{arch}/native.tar.gz') as archive:
            archive.extractall(f'target/raw/{arch}', filter='data')
    run(sys.executable, 'packaging/universal.py', 'combine', 'target/raw/aarch64/native',
        'target/raw/x86_64/native', 'target/universal')
    package('universal', args.signed, 'target/universal/sdk/prefix', 'target/universal/binaries')


def smoke(args):
    downloads = Path('target/download')
    if platform.system() == 'Darwin':
        if args.signed:
            run('bash', '.github/scripts/install_smoke_macos.sh')
            return
        packages = list(downloads.glob('*.pkg'))
        if len(packages) != 1:
            raise ValueError('Expected one unsigned installer')
        run('sudo', 'installer', '-pkg', packages[0], '-target', '/')
        installed = '/Applications/VibeCut.app'
    else:
        archives = list(downloads.glob('*.zip'))
        if len(archives) != 1:
            raise ValueError('Expected one Windows archive')
        with zipfile.ZipFile(archives[0]) as archive:
            archive.extractall('target/installed')
        installed = 'target/installed/VibeCut'
    run(sys.executable, 'packaging/distribution_smoke.py', '--installed', installed, '--output', 'target/smoke')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['native', 'universal', 'smoke', 'validate'])
    parser.add_argument('--arch', choices=['aarch64', 'x86_64', 'universal'])
    parser.add_argument('--signed', action='store_true')
    parser.add_argument('--version')
    args = parser.parse_args()
    if args.operation == 'validate':
        version = tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version']
        if args.version != version:
            raise SystemExit('Release version must match workspace.package.version')
    else:
        globals()[args.operation](args)
