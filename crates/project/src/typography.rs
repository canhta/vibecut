//! Typography fields shared by text items and text masks.

/// Regular CSS weight.
pub const DEFAULT_WEIGHT: u16 = 400;
/// Largest letter spacing magnitude in project canvas pixels.
pub const MAX_LETTER_SPACING: f32 = 1024.0;
/// Largest explicit baseline distance in project canvas pixels.
pub const MAX_LINE_HEIGHT: f32 = 16_384.0;

/// Whether a CSS weight is storable (100..=900).
pub fn valid_weight(weight: u16) -> bool {
    (100..=900).contains(&weight)
}

/// Whether a letter spacing is finite and within [`MAX_LETTER_SPACING`].
pub fn valid_letter_spacing(spacing: f32) -> bool {
    spacing.is_finite() && spacing.abs() <= MAX_LETTER_SPACING
}

/// Whether a line height is finite, not negative and within [`MAX_LINE_HEIGHT`]; zero means the
/// font's natural height.
pub fn valid_line_height(height: f32) -> bool {
    height.is_finite() && (0.0..=MAX_LINE_HEIGHT).contains(&height)
}

/// Whether a family name, weight, letter spacing and line height are storable.
pub(crate) fn valid(font: &str, weight: u16, letter_spacing: f32, line_height: f32) -> bool {
    !font.trim().is_empty()
        && font.len() <= 256
        && valid_weight(weight)
        && valid_letter_spacing(letter_spacing)
        && valid_line_height(line_height)
}
