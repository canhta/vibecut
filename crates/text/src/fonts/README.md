# Fonts

`register(FontData)` adds a font file that a catalog read, so `resolve` and `fallback_chain` find the family. The catalog supplies family, style, weight and italic; nothing is parsed from the name table. Registration is append-only for the process. Identical bytes return the existing face, a different file under the same family and style is `Conflict`, and bundled family names are reserved so bundled fallback order never changes. At most 256 faces, 32 MiB per file and 256 MiB in total are accepted, and registered bytes are retained. Registered faces join only an explicit `fallbacks` chain, never the generic bundled order. `generation()` counts registered faces; a caller that caches layouts or rasters includes it in its key. `face(id).bytes()` exposes the retained bytes read-only. Enumeration, downloads and licences of non-bundled fonts belong to [libraries](../../../libraries/src/font_catalog/README.md#font-catalog).

| Family | Faces | Source |
|---|---|---|
| Inter | Regular, Medium, SemiBold, Bold, Italic | OFL-1.1 |
| Noto Serif | Regular | OFL-1.1 |
| VibeCut Sans CJK | Regular | OFL-1.1 subset of Noto Sans SC |

The CJK face is a 2.1 MB subset covering GB2312, Big5 level 1, ASCII, CJK punctuation and fullwidth forms (9,804 characters). Characters outside the subset resolve deterministically to the preferred face and render as a missing glyph. Every asset has a licence and attribution sidecar under `assets/fonts/`. The [CJK attribution](../../../../assets/fonts/VibeCutSansCJK-Regular.otf.attribution) owns the external regeneration procedure; verify its pinned hashes before replacing the committed font. Regeneration is outside the repository build.

Fallback is deterministic. The preferred face wins when it covers the character. Otherwise the profile chain is searched in order, then the bundled table order. Whitespace and control characters stay on the preferred face.

Family identity uses Unicode lowercase consistently with the font catalog, so
accented upper/lowercase requests share resolution, deduplication and conflicts.
Style names retain ASCII-insensitive matching.
