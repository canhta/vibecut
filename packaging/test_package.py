"""Boundary checks for packaging admission without credentials or platform SDKs."""
import contextlib
import io
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

import package


class PackagingTests(unittest.TestCase):
    def test_source_archive_is_verified_before_distribution(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            prefix = base / 'prefix'
            licenses = prefix / 'compliance'
            licenses.mkdir(parents=True)
            for name in ('ffmpeg-COPYING.LGPLv2.1', 'ffmpeg-LICENSE.md', 'opus-1.0-COPYING', 'libvpx-1.0-LICENSE'):
                (licenses / name).write_text('licence fixture')
            (base / 'sources').mkdir()
            receipt = '--disable-gpl --disable-nonfree --disable-autodetect\n'
            for name in ('ffmpeg', 'opus', 'libvpx'):
                archive = base / 'sources' / f'{name}-1.0.archive'
                archive.write_bytes(b'synthetic source')
                receipt += f'{name}: 1.0\nsource: https://example.invalid/{name}\nsha256: {package.sha(archive)}\n\n'
            (prefix / 'BUILD.txt').write_text(receipt)
            import universal
            (prefix / 'lib').mkdir()
            binaries = base / 'binaries'
            binaries.mkdir()
            (binaries / 'vibecut').write_bytes(b'synthetic')
            (base / 'sources/stale-build-cache').mkdir()
            (base / 'sources/stale-build-cache/object.o').write_bytes(b'not distributable')
            with patch.object(universal, 'mac_relocate'):
                universal.export(prefix, binaries, base / 'export')
            self.assertEqual({p.name for p in (base / 'export/sdk/sources').iterdir()},
                             {f'{name}-1.0.archive' for name in ('ffmpeg', 'opus', 'libvpx')})
            (base / 'sources/opus-1.0.archive').write_bytes(b'tampered source')
            with self.assertRaisesRegex(ValueError, 'source checksum mismatch'):
                package.compliance(prefix, base / 'out')

    def test_manifest_records_symlink_without_reading_external_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'cli').symlink_to('/missing/app/Contents/MacOS/vibecut-cli')
            self.assertEqual(package.manifest(root), {'cli': {'symlink': '/missing/app/Contents/MacOS/vibecut-cli'}})

    def test_missing_licence_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory)
            (prefix / 'BUILD.txt').write_text('--disable-gpl --disable-nonfree --disable-autodetect')
            with self.assertRaisesRegex(ValueError, 'Missing licence text'):
                package.compliance(prefix, prefix / 'out')

    def test_pe_architecture_and_subsystem(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'vibecut.exe'
            data = bytearray(256)
            data[:2] = b'MZ'
            struct.pack_into('<I', data, 0x3C, 64)
            data[64:68] = b'PE\0\0'
            struct.pack_into('<H', data, 68, 0xAA64)
            struct.pack_into('<H', data, 64 + 0x5C, 2)
            path.write_bytes(data)
            self.assertEqual(package.pe_header(path), (0xAA64, 2))
            with patch.dict(package.os.environ, {'SystemRoot': directory}):
                with self.assertRaisesRegex(ValueError, 'Wrong architecture'):
                    package.windows_audit([path], 'x86_64')
            path.write_bytes(b'not an executable')
            with self.assertRaisesRegex(ValueError, 'Not a PE image'):
                package.pe_header(path)

    def test_external_macos_dependencies_are_refused(self):
        with patch.object(package, 'mac_dependencies', return_value=['/opt/local/lib/injected.dylib']):
            with self.assertRaisesRegex(ValueError, 'Unbundled dependency'):
                package.mac_relocate([Path('/app/vibecut')], [])

    def test_cache_runpaths_are_removed_and_bundle_path_is_added(self):
        calls = []
        def run(*args, **kwargs):
            calls.append(args)
            if args[:2] == ('otool', '-l'):
                return 'cmd LC_RPATH\n cmdsize 80\n path /private/cache/lib (offset 12)\n'
            return ''
        path = Path('/app/Contents/MacOS/vibecut')
        with patch.object(package, 'run', side_effect=run), patch.object(package, 'mac_dependencies', return_value=['@rpath/libavcodec.63.dylib', '/usr/lib/libSystem.B.dylib']):
            package.mac_relocate([path], [Path('/app/Contents/Frameworks/libavcodec.63.dylib')])
        self.assertIn(('install_name_tool', '-delete_rpath', '/private/cache/lib', path), calls)
        self.assertIn(('install_name_tool', '-add_rpath', '@executable_path/../Frameworks', path), calls)

    def test_private_signing_arguments_are_redacted(self):
        with patch.dict(package.os.environ, {'VIBECUT_NOTARY_KEY_ID': 'secret-fixture-id'}):
            with patch.object(package.subprocess, 'run', return_value=type('Output', (), {'stdout': ''})()):
                with contextlib.redirect_stdout(io.StringIO()) as output:
                    package.run('tool', '--key-id', 'secret-fixture-id')
        self.assertNotIn('secret-fixture-id', output.getvalue())
        self.assertNotIn('secret-fixture-id', str(package.COMMANDS))

    def test_successful_tool_output_redacts_credentials_and_team(self):
        identity = 'Developer ID Application: Test (TESTTEAM42)'
        values = {'VIBECUT_MAC_SIGN_IDENTITY': identity,
                  'VIBECUT_NOTARY_KEY_ID': 'synthetic-key-id'}
        tool_output = identity + '\nTeam: TESTTEAM42\nsynthetic-key-id\n'
        with patch.dict(package.os.environ, values):
            with patch.object(package.subprocess, 'run', return_value=type('Output', (), {'stdout': tool_output})()):
                with contextlib.redirect_stdout(io.StringIO()) as output:
                    self.assertEqual(package.run('tool'), tool_output)
        for private in (identity, 'TESTTEAM42', 'synthetic-key-id'):
            self.assertNotIn(private, output.getvalue())

class MacDeploymentTests(unittest.TestCase):
    def test_each_distribution_slice_must_match_architecture_and_deployment_floor(self):
        import macos
        for arch, slices, versions, accepted in (
                ('aarch64', 'arm64', ['11.0'], True),
                ('x86_64', 'x86_64', ['10.15'], True),
                ('universal', 'x86_64 arm64', ['11.0', '11.0'], True),
                ('universal', 'arm64', ['11.0'], False),
                ('aarch64', 'arm64', ['12.0'], False),
                ('universal', 'arm64 x86_64', ['11.0'], False),
                ('universal', 'arm64 x86_64', ['11.0', '15.0'], False)):
            with self.subTest(arch=arch, slices=slices, versions=versions):
                def run(*args):
                    if args[:2] == ('lipo', '-archs'):
                        return slices
                    if args[0] == 'otool':
                        return ''.join(f'cmd LC_BUILD_VERSION\n cmdsize 32\n platform 1\n minos {v}\n sdk 15.0\n' for v in versions) + 'cmd LC_SOURCE_VERSION\n cmdsize 16\n version 1.0\n'
                    return ''
                if accepted:
                    macos.audit(Path('app'), arch, run)
                else:
                    with self.assertRaises(ValueError):
                        macos.audit(Path('app'), arch, run)

class UniversalAdmissionTests(unittest.TestCase):
    def test_mismatched_native_inputs_are_refused_before_merging(self):
        import json
        import universal
        for mismatch in ('revision', 'sources', 'libraries'):
            with self.subTest(mismatch=mismatch), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                for architecture in ('arm', 'intel'):
                    native = root / architecture
                    (native / 'binaries').mkdir(parents=True)
                    build = {'revision': 'same', 'profile': 'release', 'command': ['cargo', 'build']}
                    if mismatch == 'revision' and architecture == 'intel':
                        build['revision'] = 'different'
                    (native / 'binaries/build.json').write_text(json.dumps(build))
                    (native / 'sdk/prefix/lib').mkdir(parents=True)
                    receipt = ''.join(f'{name}: 1.0\nsource: https://example.invalid/source\nsha256: {"0" * 64}\n' for name in ('ffmpeg', 'opus', 'libvpx'))
                    if mismatch == 'sources' and architecture == 'intel':
                        receipt = receipt.replace('ffmpeg: 1.0', 'ffmpeg: 2.0')
                    (native / 'sdk/prefix/BUILD.txt').write_text(receipt)
                    for i in range(7 if mismatch == 'libraries' and architecture == 'intel' else 6):
                        (native / f'sdk/prefix/lib/lib{i}.dylib').touch()
                with patch.object(universal, 'run') as run:
                    with self.assertRaises(ValueError):
                        universal.combine(root / 'arm', root / 'intel', root / 'out')
                    run.assert_not_called()

    def test_universal_otool_slice_headers_are_not_dependencies(self):
        output = 'app (architecture x86_64):\n\t@rpath/libavutil.61.dylib (compatibility version 1.0.0, current version 1.0.0)\napp (architecture arm64):\n\t@rpath/libavutil.61.dylib (compatibility version 1.0.0, current version 1.0.0)\n'
        with patch.object(package, 'run', return_value=output):
            self.assertEqual(package.mac_dependencies(Path('app')), ['@rpath/libavutil.61.dylib'] * 2)


if __name__ == '__main__':
    unittest.main()
