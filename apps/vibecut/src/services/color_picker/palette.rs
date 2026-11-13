//! Recommended swatches for the shared colour popover.

use project::Color;

/// Columns in the recommended grid; the grid holds three rows.
pub const RECOMMENDED_COLUMNS: usize = 7;

/// Recommended swatches as `0xRRGGBB`, row by row: light tints, mid tones, dark shades.
/// Picked by eye from the solid palette, not measured from the reference shot.
pub const RECOMMENDED: [u32; 21] = [
    0xffffff, 0xffc9c9, 0xfccee8, 0xfef3c6, 0xdcfce7, 0xdbeafe, 0xf3e8ff, //
    0x737373, 0xfb2c36, 0xf6339a, 0xfe9a00, 0x00c950, 0x2b7fff, 0x9810fa, //
    0x0a0a0a, 0x9f0712, 0xa50036, 0xa65f00, 0x0d542b, 0x1c398e, 0x59168b,
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
