#!/usr/bin/env python3
"""Replace original dylibs in a disposable app copy; never rebuild executables."""
import argparse
from pathlib import Path
import shutil
from package import mac_relocate, mac_sign

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--app-copy', type=Path, required=True)
parser.add_argument('--libraries', type=Path, required=True)
args = parser.parse_args()
app = args.app_copy.resolve()
libraries = sorted((app / 'Contents/Frameworks').glob('*.dylib'))
if len(libraries) < 6:
    parser.error('Expected a disposable VibeCut.app copy with bundled FFmpeg')
for library in libraries:
    source = args.libraries / library.name
    if not source.is_file():
        parser.error(f'Missing replacement {library.name}')
for library in libraries:
    shutil.copy2(args.libraries / library.name, library)
mac_relocate(libraries, libraries)
for path in libraries + sorted((app / 'Contents/MacOS').iterdir()):
    mac_sign(path, '-')
mac_sign(app, '-')
