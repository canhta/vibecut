#!/usr/bin/env python3
"""Regenerate the bundled common-Chinese font subset from a pinned Noto Sans SC face.

Usage:
    python3 tools/subset-cjk-font.py /path/to/NotoSansSC-Regular.otf [output.otf]

The source face comes from the Noto CJK Sans2.004 release archive
(18_NotoSansSC.zip, sha256 4d107c09ada479d3e48b6e78c83835773cbd9214bf6e12cdb7b60f8e068292ec).
The script pins fontTools, the source and output hashes, and the exact character construction, so
a mismatch is a hard failure rather than a silent asset change.
"""

from __future__ import annotations

import hashlib
import os
import sys
import tempfile
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont

FONTTOOLS_VERSION = "4.68.0"
SOURCE_SHA256 = "faa6c9df652116dde789d351359f3d7e5d2285a2b2a1f04a2d7244df706d5ea9"
OUTPUT_SHA256 = "ad6c0e9b2a8c442c8022ed3d98e51d5fca76148a45d25f2970142003ab3c4bf6"

EXPLICIT = "、。！？：；“”‘’（）《》【】…—·「」『』VibeCut越南语英语中文简体繁體"


def digest(path: Path) -> str:
    sha = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            sha.update(block)
    return sha.hexdigest()


def characters() -> str:
    chars: set[str] = set()

    def add_encoding(name: str, first: int, last: int) -> None:
        for lead in range(first, last + 1):
            for trail in range(0x40, 0x100):
                try:
                    chars.update(bytes([lead, trail]).decode(name))
                except (UnicodeDecodeError, LookupError):
                    continue

    add_encoding("gb2312", 0x81, 0xFE)
    # Big5 level 1: the common 5,401-character range.
    add_encoding("big5", 0xA4, 0xC6)
    chars.update(chr(code) for code in range(0x20, 0x7F))
    chars.update(chr(code) for code in range(0x3000, 0x3040))
    chars.update(chr(code) for code in range(0xFF00, 0xFFF0))
    chars.update(EXPLICIT)
    return "".join(sorted(chars))


def rename(font: TTFont) -> None:
    replacements = {
        1: "VibeCut Sans CJK",
        3: "VibeCut Sans CJK; subset of Noto Sans SC Regular; p0-core-04-text",
        4: "VibeCut Sans CJK Regular",
        6: "VibeCutSansCJK-Regular",
        16: "VibeCut Sans CJK",
        17: "Regular",
        18: "VibeCut Sans CJK Regular",
        21: "VibeCut Sans CJK",
        22: "Regular",
    }
    for record in font["name"].names:
        if record.nameID in replacements:
            record.string = replacements[record.nameID]


def main() -> int:
    if len(sys.argv) not in (2, 3):
        print(__doc__.strip(), file=sys.stderr)
        return 2
    if fontTools.version != FONTTOOLS_VERSION:
        print(
            f"fontTools {FONTTOOLS_VERSION} required, found {fontTools.version}",
            file=sys.stderr,
        )
        return 1
    source = Path(sys.argv[1])
    output = Path(sys.argv[2]) if len(sys.argv) == 3 else Path(
        "assets/fonts/VibeCutSansCJK-Regular.otf"
    )
    if digest(source) != SOURCE_SHA256:
        print(f"{source}: source sha256 does not match the pinned face", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory() as temporary:
        temporary_path = Path(temporary)
        text_path = temporary_path / "characters.txt"
        text_path.write_text(characters(), encoding="utf-8")
        subset_path = temporary_path / "subset.otf"
        subset.main(
            [
                str(source),
                f"--text-file={text_path}",
                "--layout-features=kern,liga,clig,vert,vrt2",
                "--glyph-names",
                "--symbol-cmap",
                "--legacy-cmap",
                "--notdef-glyph",
                "--notdef-outline",
                "--recommended-glyphs",
                "--name-IDs=*",
                "--name-legacy",
                "--name-languages=*",
                "--no-hinting",
                f"--output-file={subset_path}",
            ]
        )
        font = TTFont(subset_path)
        rename(font)
        # Keep the source timestamp so regenerated assets are bitwise reproducible.
        font.recalcTimestamp = False
        renamed = temporary_path / "renamed.otf"
        font.save(renamed)
        result = digest(renamed)
        if result != OUTPUT_SHA256:
            print(
                f"output sha256 {result} does not match pinned {OUTPUT_SHA256}",
                file=sys.stderr,
            )
            return 1
        output.parent.mkdir(parents=True, exist_ok=True)
        os.replace(renamed, output)
    print(f"wrote {output} ({digest(output)})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
