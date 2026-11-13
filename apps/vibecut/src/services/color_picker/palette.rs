//! Recommended swatches for the shared colour popover.

use project::Color;

/// Columns in the recommended grid; the grid holds three rows.
pub const RECOMMENDED_COLUMNS: usize = 7;

/// Recommended swatches as `0xRRGGBB`, row by row, measured from the centre pixel of each
/// swatch in `.spikes/web-editors/capcut-web/screens/editor-text-properties-colour-popover.png`
/// (fills are flat over a 9x9 window): columns at x 1132, 1163, 1194, 1225, 1256, 1287,
/// 1317; rows at y 699, 730, 761.
pub const RECOMMENDED: [u32; 21] = [
    0xffffff, 0xf5c1c0, 0xf6c4d9, 0xfcecc3, 0xc7f7bc, 0xb2c7fb, 0xd8c4fb, //
    0x808080, 0xe2675e, 0xed6b9d, 0xeeab3c, 0x5cbe3c, 0x6c94f8, 0xa778f7, //
    0x16161c, 0xb74239, 0x9a355b, 0x8d6520, 0x36771f, 0x2d4ea7, 0x673ab4,
];

/// The recommended swatch at `index`, if any.
pub fn recommended(index: usize) -> Option<Color> {
    RECOMMENDED.get(index).map(|value| {
        Color([
            ((value >> 16) & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            (value & 0xff) as u8,
            255,
        ])
    })
}
