# Fonts

`register(FontData)` adds a font file a catalog read (the catalog supplies family, style, weight
and italic; nothing is parsed from the name table) so `resolve` and `fallback_chain` find the
family. Registration is append-only for the process: identical bytes return the existing face,
a different file under the same family and style is `Conflict`, bundled family names are
reserved so bundled fallback order never changes, and at most 256 faces, 32 MiB per file and
256 MiB in total are accepted (registered bytes are retained). Registered faces join only an
explicit `fallbacks` chain, never the generic bundled order. `generation()` counts registered
faces; a caller that caches layouts or rasters includes it in its key (render's text cache
does). `face(id).bytes()` exposes the retained bytes read-only for a host that previews a
catalogue family in its own text stack. Enumeration, downloads and licences of non-bundled fonts belong to
[engine](../../../engine/src/libraries/font_catalog/README.md#font-catalog).

| Family | Faces | Source |
|---|---|---|
| Inter | Regular, Medium, SemiBold, Bold, Italic | OFL-1.1 |
| Noto Serif | Regular | OFL-1.1 |
| VibeCut Sans CJK | Regular | OFL-1.1 subset of Noto Sans SC |

The CJK face is a 2.1 MB subset covering GB2312, Big5 level 1, ASCII, CJK punctuation and
fullwidth forms (9,804 characters). The full Noto Sans SC Regular face is 8.3 MB; the subset
keeps the repository asset small while covering common Simplified and Traditional Chinese.
Characters outside the subset resolve deterministically to the preferred face and render as a
missing glyph. A full-font or downloadable-font follow-up can extend coverage without changing
layout logic. Every asset has a licence and attribution sidecar under `assets/fonts/`.

The [CJK attribution](../../../../assets/fonts/VibeCutSansCJK-Regular.otf.attribution) owns the
external fontTools 4.68.0 regeneration procedure: pinned Sans2.004 archive/source/output
SHA-256 values, exact GB2312 and Big5 byte ranges, Unicode ranges and explicit characters,
literal `pyftsubset` options, renamed name IDs and timestamp handling. Verify those hashes
before replacing the committed font; regeneration is outside the repository build.

Fallback is deterministic: the preferred face wins when it covers the character; otherwise the
profile chain is searched in order, then the bundled table order. Whitespace and control
characters stay on the preferred face.
