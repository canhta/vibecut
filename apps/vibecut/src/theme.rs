//! App colour tokens shared by the editor and workspace shells.
//!
//! [`docs/product/ui.md`] owns every value in the dark/light table. UI modules
//! ask this file for tokens instead of carrying colour literals.

#![allow(dead_code)] // tokens are a shared contract; later UI tickets consume some.

use crate::egui::Color32;

// Dark: brand reference surface tokens. Light: the matching reference ramp.
const WINDOW_DARK: Color32 = Color32::from_rgb(0x0F, 0x11, 0x13);
const WINDOW_LIGHT: Color32 = Color32::from_rgb(0xF4, 0xF4, 0xF4);
const PANEL_DARK: Color32 = Color32::from_rgb(0x18, 0x19, 0x1C);
const PANEL_LIGHT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const MODAL_DARK: Color32 = Color32::from_rgb(0x1C, 0x1E, 0x20);
const MODAL_LIGHT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const FIELD_DARK: Color32 = Color32::from_rgb(0x23, 0x26, 0x2A);
const FIELD_LIGHT: Color32 = Color32::from_rgb(0xED, 0xED, 0xED);
const HOVER_DARK: Color32 = Color32::from_rgb(0x2A, 0x2D, 0x32);
const HOVER_LIGHT: Color32 = Color32::from_rgb(0xE4, 0xE4, 0xE4);
const TEXT_DARK: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const TEXT_LIGHT: Color32 = Color32::from_rgb(0x13, 0x15, 0x17);
const SECONDARY_DARK: Color32 = Color32::from_rgb(0x89, 0x8A, 0x8B);
const SECONDARY_LIGHT: Color32 = Color32::from_rgb(0x56, 0x56, 0x56);
const DISABLED_DARK: Color32 = Color32::from_rgb(0x73, 0x74, 0x75);
const DISABLED_LIGHT: Color32 = Color32::from_rgb(0x7F, 0x7F, 0x7F);
const SEPARATOR_DARK: Color32 = Color32::from_rgb(0x26, 0x27, 0x2A);
const SEPARATOR_LIGHT: Color32 = Color32::from_rgb(0xE4, 0xE4, 0xE4);
const BORDER_DARK: Color32 = Color32::from_rgb(0x39, 0x3C, 0x3F);
const BORDER_LIGHT: Color32 = Color32::from_rgb(0x82, 0x82, 0x82);

/// Brand accent: selection, checked boxes, primary button and focus.
pub(crate) const ACCENT_DARK: Color32 = Color32::from_rgb(0xD1, 0xFE, 0x17);
pub(crate) const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x4F, 0x61, 0x09);

/// Labels and glyphs drawn on an accent fill.
pub(crate) const ON_ACCENT_DARK: Color32 = Color32::from_rgb(0x13, 0x15, 0x17);
pub(crate) const ON_ACCENT_LIGHT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);

/// Sparse highlight: AI markers and attention badges.
pub(crate) const HIGHLIGHT_DARK: Color32 = Color32::from_rgb(0xFF, 0x00, 0x5B);
pub(crate) const HIGHLIGHT_LIGHT: Color32 = Color32::from_rgb(0xAD, 0x00, 0x3E);

/// Error text and fill.
pub(crate) const ERROR_TEXT_DARK: Color32 = Color32::from_rgb(0xFF, 0x54, 0x62);
pub(crate) const ERROR_FILL_DARK: Color32 = Color32::from_rgb(0xE7, 0x29, 0x30);
pub(crate) const ERROR_LIGHT: Color32 = Color32::from_rgb(0xD6, 0x00, 0x16);
pub(crate) const SUCCESS_DARK: Color32 = Color32::from_rgb(0x53, 0xC5, 0x46);
pub(crate) const SUCCESS_LIGHT: Color32 = Color32::from_rgb(0x2E, 0x7D, 0x32);
pub(crate) const WARNING_DARK: Color32 = Color32::from_rgb(0xF3, 0xC9, 0x77);
pub(crate) const WARNING_LIGHT: Color32 = Color32::from_rgb(0x8A, 0x61, 0x00);

/// Video clip tokens; the table measures fill and border, not the text.
pub(crate) const VIDEO_FILL_DARK: Color32 = Color32::from_rgb(0x0A, 0x2A, 0x24);
pub(crate) const VIDEO_BORDER_DARK: Color32 = Color32::from_rgb(0x0B, 0x6B, 0x5B);
pub(crate) const VIDEO_FILL_LIGHT: Color32 = Color32::from_rgb(0xE3, 0xF7, 0xF2);
pub(crate) const VIDEO_BORDER_LIGHT: Color32 = Color32::from_rgb(0x0B, 0x6B, 0x5B);

/// Feature toolbar bar measured in Iconography; light uses the active-tab step.
const TOOLBAR_DARK: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
const TOOLBAR_LIGHT: Color32 = HOVER_LIGHT;

/// Preview well, drop scrim and the text or tint drawn on them.
const PREVIEW_BACKDROP: Color32 = Color32::BLACK;
const SCRIM: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 170);
const ON_SCRIM: Color32 = Color32::WHITE;
const IMAGE_TINT: Color32 = Color32::WHITE;

pub(crate) const fn window(dark: bool) -> Color32 {
    if dark { WINDOW_DARK } else { WINDOW_LIGHT }
}
pub(crate) const fn panel(dark: bool) -> Color32 {
    if dark { PANEL_DARK } else { PANEL_LIGHT }
}
pub(crate) const fn modal(dark: bool) -> Color32 {
    if dark { MODAL_DARK } else { MODAL_LIGHT }
}
pub(crate) const fn field(dark: bool) -> Color32 {
    if dark { FIELD_DARK } else { FIELD_LIGHT }
}
pub(crate) const fn hover(dark: bool) -> Color32 {
    if dark { HOVER_DARK } else { HOVER_LIGHT }
}
pub(crate) const fn text(dark: bool) -> Color32 {
    if dark { TEXT_DARK } else { TEXT_LIGHT }
}
pub(crate) const fn secondary(dark: bool) -> Color32 {
    if dark {
        SECONDARY_DARK
    } else {
        SECONDARY_LIGHT
    }
}
pub(crate) const fn disabled(dark: bool) -> Color32 {
    if dark { DISABLED_DARK } else { DISABLED_LIGHT }
}
pub(crate) const fn separator(dark: bool) -> Color32 {
    if dark {
        SEPARATOR_DARK
    } else {
        SEPARATOR_LIGHT
    }
}
pub(crate) const fn border(dark: bool) -> Color32 {
    if dark { BORDER_DARK } else { BORDER_LIGHT }
}
pub(crate) const fn accent(dark: bool) -> Color32 {
    if dark { ACCENT_DARK } else { ACCENT_LIGHT }
}
pub(crate) const fn on_accent(dark: bool) -> Color32 {
    if dark {
        ON_ACCENT_DARK
    } else {
        ON_ACCENT_LIGHT
    }
}
pub(crate) const fn highlight(dark: bool) -> Color32 {
    if dark {
        HIGHLIGHT_DARK
    } else {
        HIGHLIGHT_LIGHT
    }
}
pub(crate) const fn error_text(dark: bool) -> Color32 {
    if dark { ERROR_TEXT_DARK } else { ERROR_LIGHT }
}
pub(crate) const fn error_fill(dark: bool) -> Color32 {
    if dark { ERROR_FILL_DARK } else { ERROR_LIGHT }
}
pub(crate) const fn success(dark: bool) -> Color32 {
    if dark { SUCCESS_DARK } else { SUCCESS_LIGHT }
}
pub(crate) const fn warning(dark: bool) -> Color32 {
    if dark { WARNING_DARK } else { WARNING_LIGHT }
}
pub(crate) const fn video_fill(dark: bool) -> Color32 {
    if dark {
        VIDEO_FILL_DARK
    } else {
        VIDEO_FILL_LIGHT
    }
}
pub(crate) const fn video_border(dark: bool) -> Color32 {
    if dark {
        VIDEO_BORDER_DARK
    } else {
        VIDEO_BORDER_LIGHT
    }
}
pub(crate) const fn toolbar(dark: bool) -> Color32 {
    if dark { TOOLBAR_DARK } else { TOOLBAR_LIGHT }
}
pub(crate) const fn preview_backdrop() -> Color32 {
    PREVIEW_BACKDROP
}
pub(crate) const fn scrim() -> Color32 {
    SCRIM
}
pub(crate) const fn on_scrim() -> Color32 {
    ON_SCRIM
}
pub(crate) const fn image_tint() -> Color32 {
    IMAGE_TINT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_dark_and_light_tokens_are_exact() {
        let rgb = Color32::from_rgb;
        let cases = [
            (window(true), rgb(0x0F, 0x11, 0x13)),
            (window(false), rgb(0xF4, 0xF4, 0xF4)),
            (panel(true), rgb(0x18, 0x19, 0x1C)),
            (panel(false), rgb(0xFF, 0xFF, 0xFF)),
            (modal(true), rgb(0x1C, 0x1E, 0x20)),
            (field(true), rgb(0x23, 0x26, 0x2A)),
            (field(false), rgb(0xED, 0xED, 0xED)),
            (hover(true), rgb(0x2A, 0x2D, 0x32)),
            (text(true), rgb(0xFF, 0xFF, 0xFF)),
            (text(false), rgb(0x13, 0x15, 0x17)),
            (secondary(true), rgb(0x89, 0x8A, 0x8B)),
            (secondary(false), rgb(0x56, 0x56, 0x56)),
            (disabled(true), rgb(0x73, 0x74, 0x75)),
            (separator(true), rgb(0x26, 0x27, 0x2A)),
            (border(true), rgb(0x39, 0x3C, 0x3F)),
            (border(false), rgb(0x82, 0x82, 0x82)),
            (accent(true), rgb(0xD1, 0xFE, 0x17)),
            (accent(false), rgb(0x4F, 0x61, 0x09)),
            (on_accent(true), rgb(0x13, 0x15, 0x17)),
            (on_accent(false), rgb(0xFF, 0xFF, 0xFF)),
            (highlight(true), rgb(0xFF, 0x00, 0x5B)),
            (highlight(false), rgb(0xAD, 0x00, 0x3E)),
            (error_text(true), rgb(0xFF, 0x54, 0x62)),
            (error_fill(true), rgb(0xE7, 0x29, 0x30)),
            (error_text(false), rgb(0xD6, 0x00, 0x16)),
            (success(true), rgb(0x53, 0xC5, 0x46)),
            (warning(false), rgb(0x8A, 0x61, 0x00)),
            (video_fill(true), rgb(0x0A, 0x2A, 0x24)),
            (video_fill(false), rgb(0xE3, 0xF7, 0xF2)),
            (video_border(false), rgb(0x0B, 0x6B, 0x5B)),
            (toolbar(true), rgb(0x30, 0x30, 0x30)),
        ];
        for (index, (actual, expected)) in cases.into_iter().enumerate() {
            assert_eq!(actual, expected, "token case {index}");
        }
    }
}
