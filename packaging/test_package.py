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
            (base / 'sources/opus-1.0.archive').write_bytes(b'tampered source')
            with self.assertRaisesRegex(ValueError, 'source checksum mismatch'):
                package.compliance(prefix, base / 'out')

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


if __name__ == '__main__':
    unittest.main()
