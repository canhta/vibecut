//! Maps ASS style records and colours onto cue styles.

use crate::{CueStyle, HorizontalAlignment, NfcText, VerticalAlignment};

#[derive(Clone, Debug)]
pub(super) struct StyleRecord {
    pub(super) fontname: String,
    pub(super) fontsize: Option<f64>,
    pub(super) primarycolour: Option<u32>,
    pub(super) outlinecolour: Option<u32>,
    pub(super) backcolour: Option<u32>,
    pub(super) bold: Option<bool>,
    pub(super) italic: Option<bool>,
    pub(super) underline: Option<bool>,
    pub(super) strikeout: Option<bool>,
    pub(super) borderstyle: Option<f64>,
    pub(super) alignment: Option<f64>,
    pub(super) marginl: Option<f64>,
    pub(super) marginr: Option<f64>,
    pub(super) marginv: Option<f64>,
}

/// ASS colours are `&HAABBGGRR`; the alpha byte is inverted (00 opaque).
pub(super) fn color(value: &str) -> Option<u32> {
    let value = value.trim();
    let Some(value) = value
        .strip_prefix("&H")
        .or_else(|| value.strip_prefix("&h"))
    else {
        // SSA and some ASS writers store the colour as a decimal integer.
        return value
            .parse::<i64>()
            .ok()
            .and_then(|v| u32::try_from(v & 0xffff_ffff).ok());
    };
    let value = value.trim_end_matches('&');
    if value.is_empty() || value.len() > 8 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let padded = format!("{value:0>8}");
    u32::from_str_radix(&padded, 16).ok()
}

pub(super) fn rgba(value: Option<u32>, fallback: [u8; 4]) -> [u8; 4] {
    let Some(value) = value else {
        return fallback;
    };
    let alpha = (value >> 24) & 0xff;
    let blue = (value >> 16) & 0xff;
    let green = (value >> 8) & 0xff;
    let red = value & 0xff;
    [
        u8::try_from(red).unwrap_or(0),
        u8::try_from(green).unwrap_or(0),
        u8::try_from(blue).unwrap_or(0),
        255u8.saturating_sub(u8::try_from(alpha).unwrap_or(0)),
    ]
}

pub(super) fn alignment(value: f64) -> crate::CueAlignment {
    let numpad = u8::try_from((value.round() as i64).clamp(1, 9)).unwrap_or(2);
    crate::CueAlignment::from_numpad(numpad).unwrap_or_else(default_alignment)
}

fn default_alignment() -> crate::CueAlignment {
    crate::CueAlignment {
        numpad: 2,
        horizontal: HorizontalAlignment::Center,
        vertical: VerticalAlignment::Bottom,
    }
}

/// SSA `[V4 Styles]` alignment: 1-3 bottom, 5-7 top, 9-11 middle (left, centre,
/// right). Returns the equivalent numpad value; anything else is bottom centre.
pub(super) fn legacy_alignment(value: f64) -> f64 {
    match value.round() as i64 {
        v @ 1..=3 => v as f64,
        v @ 5..=7 => (v + 2) as f64,
        v @ 9..=11 => (v - 5) as f64,
        _ => 2.0,
    }
}

/// Value over script resolution, or `None` when it is not finite, negative or
/// above ten script heights.
pub(super) fn ratio(value: Option<f64>, play_res: f64) -> Option<f32> {
    value
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| (v / play_res) as f32)
        .filter(|v| v.is_finite() && (0.0..=10.0).contains(v))
}

/// [`ratio`] where a present but out-of-range value sets `clamped`.
fn checked_ratio(value: Option<f64>, play_res: f64, clamped: &mut bool) -> Option<f32> {
    let result = ratio(value, play_res);
    *clamped |= value.is_some() && result.is_none();
    result
}

/// Map a style record; the flag is set when a hostile value was replaced.
pub(super) fn map_style(
    record: &StyleRecord,
    play_res_x: f64,
    play_res_y: f64,
) -> (CueStyle, bool) {
    let mut clamped = false;
    let font_size_ratio = checked_ratio(record.fontsize, play_res_y, &mut clamped)
        .filter(|v| *v > 0.0)
        .unwrap_or_else(|| {
            clamped |= record.fontsize.is_some();
            default_style(play_res_x, play_res_y).font_size_ratio
        });
    let style = CueStyle {
        font_family: NfcText::from(if record.fontname.trim().is_empty() {
            "Arial"
        } else {
            record.fontname.trim()
        }),
        font_size_ratio,
        primary_color: rgba(record.primarycolour, [255; 4]),
        outline_color: rgba(record.outlinecolour, [0, 0, 0, 255]),
        background_color: rgba(record.backcolour, [0, 0, 0, 0]),
        border_style: record
            .borderstyle
            .map(|v| v.round() as u8)
            .filter(|v| (1..=3).contains(v))
            .unwrap_or(1),
        alignment: alignment(record.alignment.unwrap_or(2.0)),
        margin_left_ratio: checked_ratio(record.marginl, play_res_x, &mut clamped).unwrap_or(0.0),
        margin_right_ratio: checked_ratio(record.marginr, play_res_x, &mut clamped).unwrap_or(0.0),
        margin_vertical_ratio: checked_ratio(record.marginv, play_res_y, &mut clamped)
            .unwrap_or(0.0),
        bold: record.bold.unwrap_or(false),
        italic: record.italic.unwrap_or(false),
        underline: record.underline.unwrap_or(false),
        strikeout: record.strikeout.unwrap_or(false),
        max_lines: None,
        word_highlight_color: None,
    };
    (style, clamped)
}

pub(super) fn default_style(_play_res_x: f64, play_res_y: f64) -> CueStyle {
    CueStyle {
        font_family: NfcText::from("Arial"),
        font_size_ratio: (20.0 / play_res_y.max(1.0)) as f32,
        primary_color: [255; 4],
        outline_color: [0, 0, 0, 255],
        background_color: [0, 0, 0, 0],
        border_style: 1,
        alignment: default_alignment(),
        margin_left_ratio: 0.0,
        margin_right_ratio: 0.0,
        margin_vertical_ratio: 0.0,
        bold: false,
        italic: false,
        underline: false,
        strikeout: false,
        max_lines: None,
        word_highlight_color: None,
    }
}
