"""Ephemeral Developer ID keychain; never print credentials or tool output."""
import base64
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys

REQUIRED = ('MACOS_CERT_P12', 'MACOS_CERT_PASSWORD', 'APPLE_API_KEY_P8',
            'APPLE_API_KEY_ID', 'APPLE_API_ISSUER', 'APPLE_TEAM_ID')


def validate(env):
    missing = [name for name in REQUIRED if not env.get(name)]
    if missing:
        raise ValueError('Missing release secrets: ' + ', '.join(missing))


def run(*args):
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode:
        # security may repeat a password/identity in either output stream.
        raise RuntimeError(f'{Path(args[0]).name} failed (exit {result.returncode}); output suppressed')
    return result.stdout


def state_dir():
    return Path(os.environ['RUNNER_TEMP']) / 'vibecut-signing'


def cleanup():
    directory = state_dir()
    if not directory.exists():
        return
    previous = directory / 'search-list.json'
    keychain = directory / 'release.keychain-db'
    error = None
    try:
        if previous.exists():
            try:
                run('security', 'list-keychains', '-d', 'user', '-s', *json.loads(previous.read_text()))
            except Exception as failure:
                error = failure
        if keychain.exists():
            try:
                run('security', 'delete-keychain', str(keychain))
            except Exception as failure:
                error = error or failure
    finally:
        shutil.rmtree(directory)
    if error:
        raise error



def sign(prefix, binaries, output):
    validate(os.environ)
    directory = state_dir()
    directory.mkdir(mode=0o700, exist_ok=False)
    os.umask(0o077)
    try:
        keychain = directory / 'release.keychain-db'
        cert = directory / 'certificate.p12'
        cert.write_bytes(base64.b64decode(os.environ['MACOS_CERT_P12'], validate=True))
        key = directory / 'notary.p8'
        key.write_text(os.environ['APPLE_API_KEY_P8'])
        password = secrets.token_urlsafe(32)
        previous = re.findall(r'"([^"]+)"', run('security', 'list-keychains', '-d', 'user'))
        (directory / 'search-list.json').write_text(json.dumps(previous))
        run('security', 'create-keychain', '-p', password, str(keychain))
        run('security', 'set-keychain-settings', '-lut', '21600', str(keychain))
        run('security', 'unlock-keychain', '-p', password, str(keychain))
        run('security', 'list-keychains', '-d', 'user', '-s', str(keychain), *previous)
        run('security', 'import', str(cert), '-k', str(keychain), '-P',
            os.environ['MACOS_CERT_PASSWORD'], '-T', '/usr/bin/codesign', '-T', '/usr/bin/security')
        cert.unlink()
        run('security', 'set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:',
            '-s', '-k', password, str(keychain))
        identities = run('security', 'find-identity', '-v', '-p', 'codesigning', str(keychain))
        team = os.environ['APPLE_TEAM_ID']
        matches = [identity for identity in re.findall(r'"([^"]+)"', identities)
                   if identity.startswith('Developer ID Application: ') and identity.endswith(f'({team})')]
        if len(matches) != 1:
            raise ValueError('Certificate must provide exactly one valid Developer ID Application identity for APPLE_TEAM_ID')
        env = dict(os.environ, VIBECUT_MAC_SIGN_IDENTITY=matches[0],
                   VIBECUT_NOTARY_KEY_FILE=str(key),
                   VIBECUT_NOTARY_KEY_ID=os.environ['APPLE_API_KEY_ID'],
                   VIBECUT_NOTARY_ISSUER_ID=os.environ['APPLE_API_ISSUER'])
        # Capture even successful output: signing identifiers belong in neither logs nor reports.
        result = subprocess.run(['python3', 'packaging/package.py', '--os', 'macos', '--arch', 'aarch64',
                                 '--ffmpeg', prefix, '--binaries', binaries, '--output', output],
                                env=env, capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError('Signed packaging/notarization failed; output suppressed to protect credentials')
    finally:
        cleanup()


if __name__ == '__main__':
    try:
        if sys.argv[1:] == ['validate']:
            validate(os.environ)
        elif sys.argv[1:] == ['cleanup']:
            cleanup()
        elif len(sys.argv) == 5 and sys.argv[1] == 'sign':
            sign(*sys.argv[2:])
        else:
            raise ValueError('Expected validate, cleanup, or sign PREFIX BINARIES OUTPUT')
    except Exception as error:
        # Only our fixed diagnostics may reach logs; base64/path errors may contain input.
        if isinstance(error, (ValueError, RuntimeError)) and str(error).startswith(
                ('Missing release secrets:', 'Certificate must', 'Signed packaging/', 'security failed')):
            print(str(error), file=sys.stderr)
        else:
            print('Release signing setup failed; sensitive diagnostics suppressed', file=sys.stderr)
        sys.exit(1)
