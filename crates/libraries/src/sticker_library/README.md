# Sticker library

Bundled stickers for the Stickers panel. A sticker is addressed by `provider:value` (`flags:US`, `shapes:star`); the id is what projects store, with the intrinsic size captured at insert.

| Module | Responsibility | Contract |
|---|---|---|
| `flags` | Flag rows for the flag provider, in source order: 254 countries with metadata, then 17 organisations and subdivisions with empty metadata. | Countries generated from the reference dataset; 271 rows. |
| `search` | Case-insensitive flag and shape search with region and continent keywords. | Empty query returns all rows; default limit 100; `total` counts before the limit. |
| `recents` | Most recent sticker ids, newest first, deduplicated, capped at 50, kept outside projects. | Unknown ids are refused; a missing file is an empty list; malformed content is an error. |

Flag SVGs live under [assets/stickers/flags](../../../../assets/stickers/flags/README.md). Shape values name graphic definitions owned by [render](../../../render/src/scene/README.md#generated-graphic-geometry); this crate holds ids only.
