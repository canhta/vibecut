"""The distribution's deployment floor and Mach-O admission."""
import re

MINIMUM_VERSION = '11.0'


def audit(path, arch, run):
    expected = {'aarch64': {'arm64'}, 'x86_64': {'x86_64'},
                'universal': {'arm64', 'x86_64'}}[arch]
    run('lipo', '-info', path)
    arches = set(run('lipo', '-archs', path).split())
    if arches != expected:
        raise ValueError(f'Wrong architecture for {path.name}: {arches}')
    commands = run('otool', '-l', path)
    versions = re.findall(r'cmd LC_(?:BUILD_VERSION|VERSION_MIN_MACOSX)\s+cmdsize \d+\s+(?:platform \S+\s+minos|version) (\d+(?:\.\d+)+)', commands)
    # otool emits one LC_BUILD_VERSION/LC_VERSION_MIN_MACOSX per slice.
    if len(versions) != len(expected) or any(
            tuple(map(int, version.split('.')[:2])) > (11, 0) for version in versions):
        raise ValueError(f'Mach-O deployment target exceeds macOS {MINIMUM_VERSION}: {path.name}')
