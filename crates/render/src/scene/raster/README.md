# Frame rasters

Pixels the scene draws from sources that are not decoded media. `raster.rs` rasterises and caches caption and text lines. `sticker.rs` draws bundled stickers: flag SVGs from `sticker_flags.rs` (one `include_bytes!` row per file under `assets/stickers/flags`) rasterised at the destination pixel size, and shape stickers drawn through the generated graphic contours. The parent [scene](../README.md) owns placement, opacity and the sticker error contract.
