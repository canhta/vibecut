#!/usr/bin/env python3
"""Stage audited native distributions; see README.md for build and release gates."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import struct
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parent.parent
COMMANDS = []


def redact(value):
    secrets = [v for k, v in os.environ.items()
               if k.startswith(('VIBECUT_MAC_', 'VIBECUT_NOTARY_')) and v]
    # Platform tools can print the team separately from the signing identity.
    for identity in secrets.copy():
        secrets.extend(re.findall(r'\(([A-Z0-9]{10})\)', identity))
    for secret in sorted(set(secrets), key=len, reverse=True):
        value = value.replace(secret, '<maintainer Developer ID>')
    return value


def run(*args, env=None):
    args = [str(a) for a in args]
    safe = [redact(a) for a in args]
    COMMANDS.append(safe)
    print('+', json.dumps(safe), flush=True)
    result = subprocess.run(args, check=True, capture_output=True, text=True, env=env)
    if result.stdout:
        print(redact(result.stdout), end='')
    return result.stdout


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def manifest(directory):
    return {p.relative_to(directory).as_posix(): ({'symlink': os.readlink(p)} if p.is_symlink() else sha(p))
            for p in sorted(directory.rglob('*')) if p.is_file() or p.is_symlink()}


def compliance(prefix, destination):
    receipt = (prefix / 'BUILD.txt').read_text()
    for flag in ('--disable-gpl', '--disable-nonfree', '--disable-autodetect'):
        if flag not in receipt:
            raise ValueError(f'FFmpeg receipt missing {flag}')
    licenses = prefix / 'compliance'
    for pattern in ('*COPYING.LGPLv2.1', '*LICENSE.md', 'opus-*-COPYING', 'libvpx-*-LICENSE'):
        matches = list(licenses.glob(pattern))
        if not matches or any(not p.stat().st_size for p in matches):
            raise ValueError(f'Missing licence text: {pattern}')
    for name, digest in re.findall(r'^backport: (.+) sha256: ([a-f0-9]{64})$', receipt, re.M):
        if sha(licenses / 'patches' / Path(name).name) != digest:
            raise ValueError('FFmpeg backport checksum mismatch')
    shutil.copytree(licenses, destination)
    shutil.copy2(prefix / 'BUILD.txt', destination)
    shutil.copy2(ROOT / 'xtask/src/ffmpeg.rs', destination / 'build-recipe.rs')
    sources = destination / 'sources'
    sources.mkdir()
    identities = re.findall(r'^(ffmpeg|opus|libvpx): ([\d.]+)\nsource: (\S+)\nsha256: ([a-f0-9]{64})$', receipt, re.M)
    if {i[0] for i in identities} != {'ffmpeg', 'opus', 'libvpx'}:
        raise ValueError('Missing corresponding FFmpeg/Opus/libvpx source identities')
    for name, version, _, digest in identities:
        archive = prefix.parent / 'sources' / f'{name}-{version}.archive'
        if sha(archive) != digest:
            raise ValueError(f'Corresponding source checksum mismatch: {archive.name}')
        shutil.copy2(archive, sources)
    shutil.copy2(ROOT / 'LICENSE', destination / 'VibeCut-MIT.txt')
    # Include the licences of the embedded fonts, with their admitted attributions.
    for license_file in (ROOT / 'assets').rglob('*'):
        if license_file.is_file() and (license_file.name.startswith(('OFL', 'LICENSE', 'NOTICE'))
                                      or license_file.suffix == '.attribution'):
            target = destination / 'assets' / license_file.relative_to(ROOT / 'assets')
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(license_file, target)
    (destination / 'NOTICE.txt').write_text(
        'VibeCut uses dynamically linked FFmpeg under LGPL-2.1-or-later, with Opus '
        'and libvpx. Corresponding archives, backports, build receipt, recipe and '
        'licence texts accompany this package. See packaging/README.md for rebuilding '
        'and replacing libraries. Reverse engineering for debugging modifications '
        'to these libraries is permitted. No model weights are included.\n')
    shutil.copy2(ROOT / 'packaging/README.md', destination / 'README.md')


def mac_dependencies(path):
    return [line.strip().split(' (compatibility')[0]
            for line in run('otool', '-L', path).splitlines()[1:] if line.strip()]


def mac_relocate(files, libraries):
    names = {p.name for p in libraries}
    for path in files:
        if path in libraries:
            run('install_name_tool', '-id', f'@rpath/{path.name}', path)
        for dep in mac_dependencies(path):
            name = Path(dep).name
            if name in names:
                if dep != f'@rpath/{name}':
                    run('install_name_tool', '-change', dep, f'@rpath/{name}', path)
            elif not dep.startswith(('/System/Library/', '/usr/lib/')):
                raise ValueError(f'Unbundled dependency {dep} in {path.name}')
        commands = run('otool', '-l', path)
        paths = re.findall(r'cmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset', commands)
        for old in paths:
            run('install_name_tool', '-delete_rpath', old, path)
        rpath = '@loader_path' if path in libraries else '@executable_path/../Frameworks'
        run('install_name_tool', '-add_rpath', rpath, path)


def mac_sign(path, identity):
    identifier = 'com.canhta.vibecut'
    if path.name == 'vibecut-cli':
        identifier += '.cli'
    elif path.suffix == '.dylib':
        identifier += '.ffmpeg.' + path.stem
    elif path.suffix == '.dmg':
        identifier += '.diskimage'
    args = ['codesign', '--force', '--sign', identity, '--identifier', identifier]
    if identity != '-':
        args += ['--timestamp']
        if path.suffix != '.dmg':
            args += ['--options', 'runtime']
    run(*args, path)
    run('codesign', '--verify', '--strict', path)


def notarize(path):
    profile = os.environ.get('VIBECUT_NOTARY_PROFILE')
    if profile:
        credentials = ['--keychain-profile', profile]
    else:
        credentials = ['--key', os.environ['VIBECUT_NOTARY_KEY_FILE'],
                       '--key-id', os.environ['VIBECUT_NOTARY_KEY_ID'],
                       '--issuer', os.environ['VIBECUT_NOTARY_ISSUER_ID']]
    result = json.loads(run('xcrun', 'notarytool', 'submit', path, *credentials,
                            '--wait', '--output-format', 'json'))
    if result.get('status') != 'Accepted':
        raise ValueError('Notarization refused; inspect submission using notarytool log')
    return {'status': result['status']}


def pe_header(path):
    with path.open('rb') as f:
        if f.read(2) != b'MZ':
            raise ValueError(f'Not a PE image: {path}')
        f.seek(0x3C)
        offset = struct.unpack('<I', f.read(4))[0]
        f.seek(offset)
        if f.read(4) != b'PE\0\0':
            raise ValueError(f'Invalid PE signature: {path}')
        machine = struct.unpack('<H', f.read(2))[0]
        f.seek(offset + 0x5C)
        subsystem = struct.unpack('<H', f.read(2))[0]
        return machine, subsystem


def windows_audit(files, arch):
    expected = {'x86_64': 0x8664, 'aarch64': 0xAA64}[arch]
    names = {p.name.lower() for p in files}
    system = Path(os.environ['SystemRoot']) / 'System32'
    for path in files:
        machine, subsystem = pe_header(path)
        if machine != expected:
            raise ValueError(f'Wrong architecture: {path.name}')
        if path.suffix == '.exe' and subsystem != (2 if path.stem == 'vibecut' else 3):
            raise ValueError(f'Wrong executable subsystem: {path.name}')
        output = run('dumpbin', '/dependents', path)
        dependencies = re.findall(r'^\s+([\w.-]+\.dll)\s*$', output, re.M | re.I)
        if not dependencies:
            raise ValueError(f'No dependencies reported for {path.name}')
        for dep in dependencies:
            if dep.lower() not in names and not dep.lower().startswith(('api-ms-', 'ext-ms-')):
                # Never satisfy missing FFmpeg from a developer/system installation.
                if re.match(r'(avcodec|avformat|avutil|avfilter|swscale|swresample)-', dep, re.I) or not (system / dep).is_file():
                    raise ValueError(f'Unbundled dependency {dep}')


def package(args):
    native = {'Darwin': 'macos', 'Windows': 'windows'}.get(platform.system())
    if native != args.os:
        raise ValueError('Packaging and dependency auditing must run on the target OS')
    prefix = args.ffmpeg.resolve()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {'version': version, 'os': args.os, 'arch': args.arch,
               'revision': run('git', '-C', ROOT, 'rev-parse', 'HEAD').strip(),
               'release_acceptance': False, 'invocation': sys.argv, 'input_sha256': manifest(args.binaries)}
    if (args.binaries / 'build.json').is_file():
        receipt['build'] = json.loads((args.binaries / 'build.json').read_text())
    try:
        if args.os == 'macos':
            payload = output / 'root'
            app = payload / 'Applications/VibeCut.app'
            contents = app / 'Contents'
            binaries = contents / 'MacOS'
            libraries = contents / 'Frameworks'
            resources = contents / 'Resources'
            for d in (binaries, libraries, resources):
                d.mkdir(parents=True, exist_ok=True)
            for name in ('vibecut', 'vibecut-cli'):
                shutil.copy2(args.binaries / name, binaries / name)
            # Flatten versioned dylib symlinks to actual runtime filenames.
            for path in (prefix / 'lib').glob('*.dylib'):
                if not path.is_symlink():
                    name = Path(mac_dependencies(path)[0]).name
                    shutil.copy2(path, libraries / name)
            libs = sorted(libraries.glob('*.dylib'))
            if len(libs) < 6:
                raise ValueError('Incomplete FFmpeg dylib set')
            files = libs + [binaries / 'vibecut-cli', binaries / 'vibecut']
            for path in files:
                arches = run('lipo', '-archs', path).strip().split()
                if arches != [{'aarch64': 'arm64', 'x86_64': 'x86_64'}[args.arch]]:
                    raise ValueError(f'Wrong architecture for {path.name}: {arches}')
            mac_relocate(files, libs)
            shutil.copy2(ROOT / 'assets/app-icon/vibecut.icns', resources)
            with (contents / 'Info.plist').open('wb') as f:
                plistlib.dump(dict(CFBundleExecutable='vibecut', CFBundleIdentifier='com.canhta.vibecut',
                                  CFBundleName='VibeCut', CFBundlePackageType='APPL',
                                  CFBundleShortVersionString=version, CFBundleVersion=version,
                                  CFBundleIconFile='vibecut.icns', NSHighResolutionCapable=True), f)
            compliance(prefix, resources / 'compliance')
            linkdir = payload / 'usr/local/bin'
            linkdir.mkdir(parents=True)
            (linkdir / 'vibecut-cli').symlink_to('/Applications/VibeCut.app/Contents/MacOS/vibecut-cli')
            identity = os.environ.get('VIBECUT_MAC_SIGN_IDENTITY', '-')
            for path in files:
                mac_sign(path, identity)
            mac_sign(app, identity)
            receipt['signing'] = 'ad-hoc' if identity == '-' else 'Developer ID requested'
            pkg = output / f'VibeCut-{version}-macos-{args.arch}.pkg'
            command = ['pkgbuild', '--root', payload, '--identifier', 'com.canhta.vibecut',
                       '--version', version, '--install-location', '/']
            components = output / 'components.plist'
            with components.open('wb') as f:
                plistlib.dump([dict(RootRelativeBundlePath='Applications/VibeCut.app',
                                     BundleIsRelocatable=False, BundleHasStrictIdentifier=True,
                                     BundleIsVersionChecked=True, BundleOverwriteAction='upgrade')], f)
            command += ['--component-plist', components]
            installer = os.environ.get('VIBECUT_MAC_INSTALLER_IDENTITY')
            notary = bool(os.environ.get('VIBECUT_NOTARY_PROFILE') or os.environ.get('VIBECUT_NOTARY_KEY_FILE'))
            if (installer or notary) and identity == '-':
                raise ValueError('Installer signing/notarization requires app signing identity')
            if installer:
                command += ['--sign', installer]
            run(*command, pkg)
            receipt['package_payload_files'] = manifest(payload)
            if installer:
                run('pkgutil', '--check-signature', pkg)
            if notary:
                # Notarize/staple the app before sealing it into the outer image.
                archive = output / 'notary-app.zip'
                run('ditto', '-c', '-k', '--keepParent', app, archive)
                receipt['app_notarization'] = notarize(archive)
                run('xcrun', 'stapler', 'staple', app)
                run('xcrun', 'stapler', 'validate', app)
                run('spctl', '--assess', '--type', 'execute', '--verbose', app)
                image_root = output / 'image'
                image_root.mkdir()
                run('ditto', app, image_root / 'VibeCut.app')
                (image_root / 'Applications').symlink_to('/Applications')
                shutil.copy2(ROOT / 'packaging/install-cli.sh', image_root)
                shutil.copy2(ROOT / 'packaging/README.md', image_root)
                dmg = output / f'VibeCut-{version}-macos-{args.arch}.dmg'
                run('hdiutil', 'create', '-volname', 'VibeCut', '-srcfolder', image_root,
                    '-format', 'UDZO', dmg)
                mac_sign(dmg, identity)
                receipt['image_notarization'] = notarize(dmg)
                run('xcrun', 'stapler', 'staple', dmg)
                run('xcrun', 'stapler', 'validate', dmg)
                run('spctl', '--assess', '--type', 'open', '--context', 'context:primary-signature', '--verbose', dmg)
                receipt['image_sha256'] = sha(dmg)
                if installer:
                    receipt['package_notarization'] = notarize(pkg)
                    run('xcrun', 'stapler', 'staple', pkg)
                    run('xcrun', 'stapler', 'validate', pkg)
            receipt['package_sha256'] = sha(pkg)
            receipt['package_signing'] = 'Developer ID Installer' if installer else 'unsigned; local test only'
        else:
            payload = output / 'VibeCut'
            payload.mkdir()
            for name in ('vibecut.exe', 'vibecut-cli.exe'):
                shutil.copy2(args.binaries / name, payload)
            for path in (prefix / 'bin').glob('*.dll'):
                shutil.copy2(path, payload)
            files = sorted(payload.iterdir())
            if len(files) < 8:
                raise ValueError('Incomplete FFmpeg DLL set')
            windows_audit(files, args.arch)
            compliance(prefix, payload / 'compliance')
            for name in ('install.ps1', 'uninstall.ps1'):
                shutil.copy2(ROOT / 'packaging/windows' / name, payload)
            certificate = os.environ.get('VIBECUT_WINDOWS_CERT_SHA1')
            if certificate:
                timestamp = os.environ['VIBECUT_WINDOWS_TIMESTAMP_URL']
                # Libraries first, CLI helper, app, then installer scripts.
                for path in sorted(payload.glob('*.dll')) + [payload / 'vibecut-cli.exe', payload / 'vibecut.exe']:
                    run('signtool', 'sign', '/sha1', certificate, '/fd', 'SHA256', '/tr', timestamp, '/td', 'SHA256', path)
                    run('signtool', 'verify', '/pa', '/all', path)
                for name in ('uninstall.ps1', 'install.ps1'):
                    run('powershell', '-NoProfile', '-File', ROOT / 'packaging/windows/sign-script.ps1', payload / name, certificate, timestamp)
            receipt['signing'] = 'requested' if certificate else 'unsigned'
            (payload / 'install-manifest.json').write_text(json.dumps(manifest(payload), indent=2) + '\n')
            archive = shutil.make_archive(str(output / f'VibeCut-{version}-windows-{args.arch}'), 'zip', output, 'VibeCut')
            receipt['package_sha256'] = sha(Path(archive))
        receipt['files'] = manifest(payload)
        receipt['status'] = 'staged; release checks remain mandatory'
    except Exception:
        receipt['status'] = 'failed; do not distribute'
        raise
    finally:
        receipt['commands'] = COMMANDS
        (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--os', choices=['macos', 'windows'], required=True)
    parser.add_argument('--arch', choices=['aarch64', 'x86_64'], required=True)
    parser.add_argument('--binaries', type=Path, required=True)
    parser.add_argument('--ffmpeg', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='New directory; never overwritten')
    package(parser.parse_args())


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError, KeyError) as error:
        print('Packaging failed; inspect the local receipt and tool diagnostics.', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError):
            print(redact(error.stderr or ''), file=sys.stderr)
        else:
            print(str(error), file=sys.stderr)
        sys.exit(1)
