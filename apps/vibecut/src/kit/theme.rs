//! Owns the app design tokens.
//!
//! Canonical app palette and measured layout tokens. `docs/product/ui/tokens.md`
//! indexes the geometry provenance; reference colours are not adopted here.

#![allow(dead_code)] // tokens are a shared contract; later UI tickets consume some.

use eframe::egui::Color32;

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
        ];
        for (index, (actual, expected)) in cases.into_iter().enumerate() {
            assert_eq!(actual, expected, "token case {index}");
        }
    }
}

/// Measured space scale: outer/field insets, tab padding and panel gutters.
pub(crate) mod space {
    pub const HAIRLINE: f32 = 1.0;
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const ICON_LABEL: f32 = 5.0;
    pub const SM: f32 = 6.0;
    pub const PANEL_GUTTER: f32 = 7.0;
    pub const MD: f32 = 8.0;
    pub const ROW_INSET: f32 = 10.0;
    pub const LG: f32 = 12.0;
    pub const GRID: f32 = 15.0;
    pub const XL: f32 = 16.0;
    pub const TAB_GAP: f32 = 28.0;
}
/// Only panel and control corners are measured; dialog corners remain proposed.
pub(crate) mod radius {
    pub const PANEL: u8 = 0;
    pub const FRAME_PANEL: u8 = 8;
    pub const CONTROL: u8 = 4;
    /// Value fields and steppers (221643 corners: about 2 px; the 4 px mapping was unproven).
    pub const FIELD: u8 = 2;
    pub const UPLOAD: u8 = 8;
}
/// Inter-equivalent sizes inferred from measured capital heights.
pub(crate) mod type_size {
    pub const BADGE: f32 = 10.0;
    pub const PLAYER_VALUE: f32 = 11.0;
    pub const PLAYER_TITLE: f32 = 11.0;
    pub const SMALL_LABEL: f32 = 11.0;
    pub const BODY: f32 = 12.0;
    pub const CHROME: f32 = 14.0;
    /// Ruler captions: digit height 8 px in 221643.
    pub const CAPTION: f32 = 11.0;
}
/// Measured boxes and row pitches from the desktop capture set.
pub(crate) mod size {
    pub const PLAYER_FOOTER: f32 = 44.0;
    pub const PLAYER_FIT_ROW: f32 = 26.0;
    pub const PLAYER_FIT_OFFSET: f32 = 24.0;
    pub const PLAYER_TIMECODE: f32 = 64.0;
    pub const PLAYER_SCENES_X: f32 = 171.0;
    pub const PLAYER_RATIO_X: f32 = 49.5;
    pub const PLAYER_RATIO_WIDTH: f32 = 36.0;
    pub const PLAYER_ZOOM_X: f32 = 86.0;
    pub const PLAYER_QUALITY_X: f32 = 117.0;
    pub const TOPBAR: f32 = 36.0;
    pub const TAB_STRIP: f32 = 42.0;
    pub const TOOLBAR: f32 = 32.0;
    pub const FIELD: f32 = 22.0;
    pub const CONTROL: f32 = 24.0;
    pub const PROPERTY_ROW: f32 = 39.0;
    pub const SECTION: f32 = 55.0;
    pub const SHORTCUT_ROW: f32 = 40.0;
    pub const VIDEO_TRACK: f32 = 76.0;
    pub const CLIP: f32 = 74.0;
    /// Top, filmstrip and waveform bands inside the 74 px clip (221643 y 787-858).
    pub const CLIP_BAND: f32 = 17.0;
    pub const CLIP_FILMSTRIP: f32 = 39.0;
    pub const CLIP_WAVEFORM: f32 = 16.0;
    pub const TRACK_LABEL: f32 = 123.0;
    pub const SCROLLBAR: f32 = 6.0;
    /// Ruler strip under the toolbar (221643 y 607-629); lanes start at y 635, 28 px below its top.
    pub const RULER: f32 = 22.0;
    /// Empty band between the ruler and the track lanes (221643 y 629-635).
    pub const RULER_GAP: f32 = 6.0;
    /// Strip under the lanes that holds the 6 px horizontal scrollbar (221643 y 1009-1021).
    pub const TIMELINE_FOOT: f32 = 12.0;
    /// Playhead handle in the ruler (221643 x 356-365, y 607-621).
    pub const PLAYHEAD_HANDLE_WIDTH: f32 = 10.0;
    pub const PLAYHEAD_HANDLE_HEIGHT: f32 = 15.0;
    /// Pitch of the track header controls (221643 glyph centres 22/47/71/95/119).
    pub const HEADER_CELL: f32 = 24.0;
    pub const SUBNAV: f32 = 120.0;
    pub const SUBNAV_BUTTON: f32 = 104.0;
    pub const TAB_MIN: f32 = 48.0;
    pub const FEATURE_TAB_PITCH: f32 = 49.0;
    pub const SEGMENT_WIDTH: f32 = 115.0;
    pub const STEPPER: f32 = 16.0;
    pub const VALUE: f32 = 54.0;
    pub const KEYFRAME: f32 = 27.0;
    pub const KEYFRAME_DIAMOND: f32 = 10.0;
    pub const POSITION: f32 = 66.5;
    pub const SLIDER: f32 = 336.0;
    pub const SLIDER_RAIL: f32 = 2.0;
    pub const SLIDER_THUMB_WIDTH: f32 = 8.0;
    pub const SLIDER_THUMB_HEIGHT: f32 = 12.0;
    pub const ZOOM_SLIDER: f32 = 96.0;
    /// Zoom slider thumb is 6 px wide (221643 x 1632-1637, y 584-595); the 8 px thumb is the properties slider.
    pub const ZOOM_THUMB_WIDTH: f32 = 8.0;
    pub const MEDIA_THUMB_WIDTH: f32 = 120.0;
    pub const MEDIA_THUMB_HEIGHT: f32 = 73.0;
    pub const MEDIA_LABEL_HEIGHT: f32 = 16.0;
    pub const AUDIO_ROW: f32 = 48.0;
    pub const EFFECT_CARD: f32 = 88.0;
    pub const EFFECT_THUMB_HEIGHT: f32 = 86.0;
    pub const EFFECT_PITCH: f32 = 116.0;
    pub const SHORTCUT_BUTTON: f32 = 72.0;
    pub const TOGGLE_WIDTH: f32 = 36.0;
    pub const TOGGLE_HEIGHT: f32 = 16.0;
    pub const EXPORT_WIDTH: f32 = 67.0;
    /// Selected-item box stroke and handle ring (reference capture 2026-11-06).
    pub const TRANSFORM_STROKE: f32 = 2.0;
    /// Corner and side handle face; the reference draws a 10 px circle.
    pub const TRANSFORM_HANDLE: f32 = 10.0;
    /// Pointer target around each handle, larger than its face.
    pub const TRANSFORM_HIT: f32 = 9.0;
    /// Rotation grip circle and its constant screen gap below the box.
    pub const TRANSFORM_GRIP: f32 = 18.0;
    pub const TRANSFORM_GAP: f32 = 12.0;
}
/// Measured glyph boxes; larger empty-state glyphs are not editor measurements.
pub(crate) mod icon {
    pub const DENSE: f32 = 16.0;
    pub const PLAY: f32 = 12.0;
    pub const LAYOUT_WIDTH: f32 = 18.0;
}
/// Header and field roles reuse the app palette.
pub(crate) fn strip(dark: bool) -> Color32 {
    hover(dark)
}
pub(crate) fn bright(dark: bool) -> Color32 {
    text(dark)
}
pub(crate) fn track_rest(dark: bool) -> Color32 {
    border(dark)
}
pub(crate) fn waveform(dark: bool) -> Color32 {
    if dark {
        VIDEO_BORDER_DARK
    } else {
        VIDEO_BORDER_LIGHT
    }
}
/// Measured label weight, using the already bundled font.
pub(crate) fn label_font() -> eframe::egui::FontId {
    eframe::egui::FontId::new(
        type_size::BODY,
        eframe::egui::FontFamily::Name("Inter-SemiBold".into()),
    )
}
pub(crate) fn disabled_icon(dark: bool) -> Color32 {
    disabled(dark)
}
