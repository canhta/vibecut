# Sticker pixels

`sticker.rs` rasterises one sticker to a premultiplied RGBA8 raster: a `flags:<code>` sticker from the bundled SVG table, a `shapes:<value>` sticker through the graphic contours, and a placeholder box for an asset that cannot be drawn. The behaviour, the placement rules and the typed errors are in [the scene contract](../README.md).

`flags` owns the sorted `FLAGS` table of `include_bytes!` rows under `assets/stickers/flags`. The directory listing owns the id list; a test in `flags.rs` and one in `libraries` check both tables against it, and `cargo xtask assets` checks that every file has its sidecar.
