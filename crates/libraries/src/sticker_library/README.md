# Sticker library

Bundled stickers for the Stickers panel. A sticker is addressed by `flags:US`, a `provider:value` id; the id is what projects store, with the intrinsic size captured at insert. Stickers are flags only. Shapes are graphic items (render's generated graphic definitions) inserted by their `graphic.insert.*` commands, and this crate knows nothing of them.

| Module | Responsibility | Contract |
|---|---|---|
| `flags` | Flag rows for the flag provider, in source order: 254 countries with metadata, then 17 organisations and subdivisions with empty metadata. | Countries generated from the reference dataset; 271 rows. |
| `search` | Case-insensitive flag search with region and continent keywords. | Empty query returns all rows; default limit 100; `total` counts before the limit. |
| `catalog` | Control adapter: browse and search pages, id resolution with the intrinsic size (flags 640x480), and `insert_sticker`, the edit a panel or the CLI applies. | Every category lists flags; `total` counts before the limit; unknown and non-flag ids are refused. |
| `recents` | Recent ids, newest first, deduplicated, capped at 50, kept outside projects. The list is opaque: flag ids and shape command ids. | Empty ids are refused; `from_ids` drops empty ids, duplicates and anything past the cap. The app drops ids that no longer resolve and persists the list as `Preferences::sticker_recents`. |

Flag SVGs live under [assets/stickers/flags](../../../../assets/stickers/flags/README.md).
