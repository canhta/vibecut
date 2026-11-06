//! Forgiving Advanced SubStation Alpha (ASS/SSA) import.
//!
//! The parser is deliberately a text interpreter, not a renderer: script
//! metadata, styles and dialogue become typed cues, while transform and
//! animation tags are stripped with bounded warnings. No project or renderer
//! type is touched here.

use crate::{CueStyle, Error, HorizontalAlignment, NfcText, VerticalAlignment, WarningKind};
use time::{Rounding, SourceRange, Tick};

/// Largest accepted ASS document, matching the interchange reader.
pub const MAX_ASS_BYTES: usize = 16 * 1024 * 1024;
/// Project cues per caption item.
pub const MAX_ASS_CUES: usize = 100_000;
/// Bound on distinct style records.
pub const MAX_ASS_STYLES: usize = 10_000;
/// Bound on retained warnings; the rest are counted.
pub const MAX_ASS_WARNINGS: usize = 1_000;
/// Bound on one stored text value.
pub const MAX_ASS_TEXT_BYTES: usize = 1 << 20;

const DEFAULT_PLAY_RES_X: f64 = 384.0;
const DEFAULT_PLAY_RES_Y: f64 = 288.0;
const MAX_PLAY_RES: f64 = 1_000_000.0;

/// One skipped or degraded input line. `line` is one-based; zero means the
/// document as a whole.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub struct AssWarning {
    pub line: usize,
    pub kind: AssWarningKind,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AssWarningKind {
    Bom,
    Encoding,
    LineEndings,
    Replacement,
    InvalidScriptInfo,
    MalformedStyle,
    MissingStyleFormat,
    MalformedCue,
    EmptyText,
    MissingStyle,
    UnsupportedOverride,
    UnsupportedEvent,
    TruncatedOverride,
    Limit,
}
impl AssWarningKind {
    /// Stable machine label shared by CLI output and tests.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bom => "bom",
            Self::Encoding => "encoding",
            Self::LineEndings => "line_endings",
            Self::Replacement => "replacement",
            Self::InvalidScriptInfo => "invalid_script_info",
            Self::MalformedStyle => "malformed_style",
            Self::MissingStyleFormat => "missing_style_format",
            Self::MalformedCue => "malformed_cue",
            Self::EmptyText => "empty_text",
            Self::MissingStyle => "missing_style",
            Self::UnsupportedOverride => "unsupported_override",
            Self::UnsupportedEvent => "unsupported_event",
            Self::TruncatedOverride => "truncated_override",
            Self::Limit => "limit",
        }
    }
}

/// One parsed dialogue event.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssCue {
    pub range: SourceRange,
    pub text: NfcText,
    pub style: CueStyle,
}

/// Counts and warnings from a successful import. Parsing never aborts on one
/// bad line; only document-size and collection ceilings return an error.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssImport {
    pub cues: Vec<AssCue>,
    pub styles: usize,
    pub skipped_cues: usize,
    pub skipped_styles: usize,
    pub play_res_x: u32,
    pub play_res_y: u32,
    pub warnings: Vec<AssWarning>,
    /// Warnings beyond [`MAX_ASS_WARNINGS`].
    pub suppressed_warnings: usize,
}

#[derive(Clone, Debug)]
struct StyleRecord {
    fontname: String,
    fontsize: Option<f64>,
    primarycolour: Option<u32>,
    outlinecolour: Option<u32>,
    backcolour: Option<u32>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strikeout: Option<bool>,
    borderstyle: Option<f64>,
    alignment: Option<f64>,
    marginl: Option<f64>,
    marginr: Option<f64>,
    marginv: Option<f64>,
}

/// Parse an ASS/SSA document. SRT/VTT remain owned by [`crate::parse`].
///
/// Encoding follows the shared subtitle reader: UTF-8 (with or without BOM),
/// UTF-16LE/BE with BOM, and Windows-1252 fallback. Malformed cues and styles
/// are skipped with positional warnings; collection ceilings return
/// [`Error::TooLarge`].
pub fn parse_ass(bytes: &[u8]) -> Result<AssImport, Error> {
    if bytes.len() > MAX_ASS_BYTES {
        return Err(Error::TooLarge);
    }
    let mut warnings = Vec::new();
    let mut suppressed = 0usize;
    let text = decode_document(bytes, &mut warnings, &mut suppressed)?;
    let mut play_res_x = DEFAULT_PLAY_RES_X;
    let mut play_res_y = DEFAULT_PLAY_RES_Y;

    let mut section = Section::Other;
    // `[V4 Styles]` (SSA) numbers alignment differently from `[V4+ Styles]`.
    let mut legacy_styles = false;
    let mut style_format: Option<Vec<String>> = None;
    let mut event_format: Option<Vec<String>> = None;
    let mut styles: std::collections::BTreeMap<String, StyleRecord> =
        std::collections::BTreeMap::new();
    let mut skipped_styles = 0usize;
    let mut cues = Vec::new();
    let mut skipped_cues = 0usize;

    for (index, raw) in text.split('\n').enumerate() {
        let line_number = index.saturating_add(1);
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.len() > MAX_ASS_TEXT_BYTES {
            warn(
                &mut warnings,
                &mut suppressed,
                line_number,
                AssWarningKind::Limit,
            );
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            section = Section::from_name(name.trim());
            legacy_styles = name.trim().eq_ignore_ascii_case("v4 styles");
            continue;
        }
        match section {
            Section::ScriptInfo => {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                let parsed = value.trim().parse::<f64>().ok();
                let value = parsed.filter(|v| v.is_finite() && *v > 0.0 && *v <= MAX_PLAY_RES);
                match key.trim().to_ascii_lowercase().as_str() {
                    "playresx" => {
                        if let Some(value) = value {
                            play_res_x = value;
                        } else if value.is_none() {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::InvalidScriptInfo,
                            );
                        }
                    }
                    "playresy" => {
                        if let Some(value) = value {
                            play_res_y = value;
                        } else {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::InvalidScriptInfo,
                            );
                        }
                    }
                    _ => {}
                }
            }
            Section::Styles => {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                match key.trim().to_ascii_lowercase().as_str() {
                    "format" => {
                        let fields = format_fields(value);
                        if fields.is_empty() {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedStyle,
                            );
                        } else {
                            style_format = Some(fields);
                        }
                    }
                    "style" => {
                        let Some(fields) = style_format.as_ref() else {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MissingStyleFormat,
                            );
                            skipped_styles = skipped_styles.saturating_add(1);
                            continue;
                        };
                        let Some(values) = split_fields(value, fields.len()) else {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedStyle,
                            );
                            skipped_styles = skipped_styles.saturating_add(1);
                            continue;
                        };
                        let mut record = StyleRecord {
                            fontname: String::new(),
                            fontsize: None,
                            primarycolour: None,
                            outlinecolour: None,
                            backcolour: None,
                            bold: None,
                            italic: None,
                            underline: None,
                            strikeout: None,
                            borderstyle: None,
                            alignment: None,
                            marginl: None,
                            marginr: None,
                            marginv: None,
                        };
                        let mut name = None;
                        let mut bad = false;
                        for (field, value) in fields.iter().zip(values.iter()) {
                            let value = value.trim();
                            match field.as_str() {
                                "name" => name = Some(value.to_owned()),
                                "fontname" => record.fontname = value.to_owned(),
                                "fontsize" => {
                                    record.fontsize = track(number(value), value, &mut bad)
                                }
                                "primarycolour" => {
                                    record.primarycolour = track(color(value), value, &mut bad);
                                }
                                // SSA calls the outline colour "Tertiary".
                                "outlinecolour" | "tertiarycolour" => {
                                    record.outlinecolour = track(color(value), value, &mut bad);
                                }
                                "backcolour" => {
                                    record.backcolour = track(color(value), value, &mut bad);
                                }
                                "bold" => record.bold = track(boolean(value), value, &mut bad),
                                "italic" => record.italic = track(boolean(value), value, &mut bad),
                                "underline" => {
                                    record.underline = track(boolean(value), value, &mut bad);
                                }
                                "strikeout" => {
                                    record.strikeout = track(boolean(value), value, &mut bad);
                                }
                                "borderstyle" => {
                                    record.borderstyle = track(number(value), value, &mut bad);
                                }
                                "alignment" => {
                                    record.alignment =
                                        track(number(value), value, &mut bad).map(|v| {
                                            if legacy_styles {
                                                legacy_alignment(v)
                                            } else {
                                                v
                                            }
                                        });
                                }
                                "marginl" => record.marginl = track(number(value), value, &mut bad),
                                "marginr" => record.marginr = track(number(value), value, &mut bad),
                                "marginv" => record.marginv = track(number(value), value, &mut bad),
                                _ => {}
                            }
                        }
                        if bad {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedStyle,
                            );
                        }
                        let Some(name) =
                            name.filter(|name| !name.trim().is_empty() && name.len() <= 256)
                        else {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedStyle,
                            );
                            skipped_styles = skipped_styles.saturating_add(1);
                            continue;
                        };
                        if styles.len() >= MAX_ASS_STYLES
                            && !styles.contains_key(&name.to_ascii_lowercase())
                        {
                            return Err(Error::TooLarge);
                        }
                        if record.fontname.len() > 256 {
                            record.fontname.truncate(256);
                        }
                        styles.insert(name.to_ascii_lowercase(), record);
                    }
                    _ => {}
                }
            }
            Section::Events => {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                match key.trim().to_ascii_lowercase().as_str() {
                    "format" => {
                        let fields = format_fields(value);
                        if fields.is_empty() {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedCue,
                            );
                        } else {
                            event_format = Some(fields);
                        }
                    }
                    "dialogue" => {
                        let Some(fields) = event_format.as_ref() else {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedCue,
                            );
                            skipped_cues = skipped_cues.saturating_add(1);
                            continue;
                        };
                        let Some(values) = split_fields(value, fields.len()) else {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedCue,
                            );
                            skipped_cues = skipped_cues.saturating_add(1);
                            continue;
                        };
                        let mut record = EventRecord::default();
                        for (field, value) in fields.iter().zip(values.iter()) {
                            match field.as_str() {
                                "start" => record.start = value,
                                "end" => record.end = value,
                                "style" => record.style = value,
                                "text" => record.text = value,
                                "effect" => record.effect = value,
                                _ => {}
                            }
                        }
                        let Some(start) = timestamp(record.start) else {
                            skipped_cues = skipped_cues.saturating_add(1);
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedCue,
                            );
                            continue;
                        };
                        let Some(end) = timestamp(record.end) else {
                            skipped_cues = skipped_cues.saturating_add(1);
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedCue,
                            );
                            continue;
                        };
                        if end <= start {
                            skipped_cues = skipped_cues.saturating_add(1);
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedCue,
                            );
                            continue;
                        }
                        if cues.len() >= MAX_ASS_CUES {
                            return Err(Error::TooLarge);
                        }
                        let stripped = strip_text(record.text);
                        if stripped.unsupported {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::UnsupportedOverride,
                            );
                        }
                        if stripped.truncated {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::TruncatedOverride,
                            );
                        }
                        if stripped.text.trim().is_empty() {
                            skipped_cues = skipped_cues.saturating_add(1);
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::EmptyText,
                            );
                            continue;
                        }
                        if !record.effect.trim().is_empty() {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::UnsupportedOverride,
                            );
                        }
                        let requested = record.style.trim().to_ascii_lowercase();
                        let base = if requested.is_empty() {
                            styles.get("default")
                        } else {
                            styles.get(&requested)
                        };
                        if !requested.is_empty() && base.is_none() {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MissingStyle,
                            );
                        }
                        let (mut style, clamped) = base.map_or_else(
                            || (default_style(play_res_x, play_res_y), false),
                            |record| map_style(record, play_res_x, play_res_y),
                        );
                        if clamped {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::Limit,
                            );
                        }
                        if let Some(overrides) = stripped.overrides {
                            apply_overrides(
                                &mut style,
                                &overrides,
                                play_res_x,
                                play_res_y,
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                            );
                        }
                        let Ok(range) = SourceRange::new(start, end) else {
                            skipped_cues = skipped_cues.saturating_add(1);
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedCue,
                            );
                            continue;
                        };
                        if style.font_family.as_str().len() > 256 {
                            style.font_family = NfcText::from("Arial");
                        }
                        if !style.is_valid() {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::MalformedCue,
                            );
                            style = default_style(play_res_x, play_res_y);
                        }
                        cues.push(AssCue {
                            range,
                            text: NfcText::from(stripped.text),
                            style,
                        });
                    }
                    "comment" => {}
                    _ => {
                        if line.contains(':') {
                            warn(
                                &mut warnings,
                                &mut suppressed,
                                line_number,
                                AssWarningKind::UnsupportedEvent,
                            );
                        }
                    }
                }
            }
            Section::Other => {}
        }
    }
    Ok(AssImport {
        cues,
        styles: styles.len(),
        skipped_cues,
        skipped_styles,
        play_res_x: play_res_x.round().clamp(1.0, MAX_PLAY_RES) as u32,
        play_res_y: play_res_y.round().clamp(1.0, MAX_PLAY_RES) as u32,
        warnings,
        suppressed_warnings: suppressed,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    ScriptInfo,
    Styles,
    Events,
    Other,
}
impl Section {
    fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "script info" => Self::ScriptInfo,
            "v4 styles" | "v4+ styles" | "v4 styles+" => Self::Styles,
            "events" => Self::Events,
            _ => Self::Other,
        }
    }
}

#[derive(Default)]
struct EventRecord<'a> {
    start: &'a str,
    end: &'a str,
    style: &'a str,
    text: &'a str,
    effect: &'a str,
}

struct StrippedText {
    text: String,
    overrides: Option<Vec<String>>,
    unsupported: bool,
    truncated: bool,
}

fn decode_document(
    bytes: &[u8],
    warnings: &mut Vec<AssWarning>,
    suppressed: &mut usize,
) -> Result<String, Error> {
    let mut inner = Vec::new();
    let text = super::decode(bytes, &mut inner)?;
    for warning in inner {
        let kind = match warning.kind {
            WarningKind::Bom => AssWarningKind::Bom,
            WarningKind::Encoding => AssWarningKind::Encoding,
            WarningKind::LineEndings => AssWarningKind::LineEndings,
            WarningKind::Replacement => AssWarningKind::Replacement,
            _ => continue,
        };
        warn(warnings, suppressed, warning.position, kind);
    }
    Ok(text)
}

fn warn(warnings: &mut Vec<AssWarning>, suppressed: &mut usize, line: usize, kind: AssWarningKind) {
    if warnings.len() < MAX_ASS_WARNINGS {
        warnings.push(AssWarning { line, kind });
    } else {
        *suppressed = suppressed.saturating_add(1);
    }
}

fn format_fields(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|field| field.trim().to_ascii_lowercase())
        .filter(|field| !field.is_empty() && field.len() <= 64)
        .take(256)
        .collect()
}

/// Split a comma-separated record where the final field may itself contain
/// commas (ASS dialogue text). Returns `None` on a short record.
fn split_fields(value: &str, expected: usize) -> Option<Vec<&str>> {
    if expected == 0 {
        return None;
    }
    if expected == 1 {
        return Some(vec![value]);
    }
    let mut fields = Vec::with_capacity(expected);
    let mut rest = value;
    for _ in 1..expected {
        let (field, tail) = rest.split_once(',')?;
        fields.push(field);
        rest = tail;
    }
    fields.push(rest);
    Some(fields)
}

/// Flag a non-empty field that failed to parse; an empty field is just absent.
fn track<T>(parsed: Option<T>, raw: &str, bad: &mut bool) -> Option<T> {
    *bad |= parsed.is_none() && !raw.is_empty();
    parsed
}

fn number(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn boolean(value: &str) -> Option<bool> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    value.parse::<i64>().ok().map(|v| v != 0)
}

/// ASS colours are `&HAABBGGRR`; the alpha byte is inverted (00 opaque).
fn color(value: &str) -> Option<u32> {
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

fn rgba(value: Option<u32>, fallback: [u8; 4]) -> [u8; 4] {
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

fn alignment(value: f64) -> crate::CueAlignment {
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
fn legacy_alignment(value: f64) -> f64 {
    match value.round() as i64 {
        v @ 1..=3 => v as f64,
        v @ 5..=7 => (v + 2) as f64,
        v @ 9..=11 => (v - 5) as f64,
        _ => 2.0,
    }
}

/// Value over script resolution, or `None` when it is not finite, negative or
/// above ten script heights.
fn ratio(value: Option<f64>, play_res: f64) -> Option<f32> {
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
fn map_style(record: &StyleRecord, play_res_x: f64, play_res_y: f64) -> (CueStyle, bool) {
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
    };
    (style, clamped)
}

fn default_style(_play_res_x: f64, play_res_y: f64) -> CueStyle {
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
    }
}

fn timestamp(value: &str) -> Option<Tick> {
    let value = value.trim();
    let mut parts = value.split(':');
    let hours = parts.next()?.parse::<i64>().ok()?;
    let minutes = parts.next()?.parse::<i64>().ok()?;
    let seconds = parts.next()?;
    if parts.next().is_some() || hours < 0 || !(0..60).contains(&minutes) {
        return None;
    }
    let (seconds, fraction) = seconds
        .split_once('.')
        .or_else(|| seconds.split_once(','))
        .unwrap_or((seconds, "0"));
    let seconds = seconds.parse::<i64>().ok()?;
    if !(0..60).contains(&seconds) || fraction.is_empty() || fraction.len() > 3 {
        return None;
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = u32::try_from(fraction.len()).ok()?;
    let fraction = fraction.parse::<i64>().ok()?;
    let scale = 10_i64.checked_pow(digits)?;
    let millis = fraction.checked_mul(1000)?.checked_div(scale)?;
    let total = hours
        .checked_mul(3600)?
        .checked_add(minutes.checked_mul(60)?)?
        .checked_add(seconds)?
        .checked_mul(1000)?
        .checked_add(millis)?;
    Tick::from_units(total, 1000, Rounding::Nearest).ok()
}

fn strip_text(input: &str) -> StrippedText {
    let mut text = String::with_capacity(input.len());
    let mut overrides = Vec::new();
    let mut unsupported = false;
    let mut truncated = false;
    let mut drawing = false;
    let mut chars = input.chars().peekable();
    while let Some(character) = chars.next() {
        // Vector drawing commands (`\p1`) are shapes, not caption text.
        if drawing && character != '{' {
            continue;
        }
        match character {
            '{' => {
                let mut block = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    block.push(next);
                    if block.len() > MAX_ASS_TEXT_BYTES {
                        break;
                    }
                }
                if !closed {
                    truncated = true;
                    break;
                }
                if block.len() > MAX_ASS_TEXT_BYTES {
                    truncated = true;
                    continue;
                }
                if let Some(on) = drawing_mode(&block) {
                    drawing = on;
                }
                if !block.is_empty() {
                    overrides.push(block);
                }
            }
            '\\' => match chars.peek().copied() {
                Some('N') | Some('n') => {
                    let _ = chars.next();
                    text.push('\n');
                }
                Some('h') => {
                    let _ = chars.next();
                    text.push(' ');
                }
                Some('\\') => {
                    let _ = chars.next();
                    text.push('\\');
                }
                Some('{') => {
                    let _ = chars.next();
                    text.push('{');
                }
                Some('}') => {
                    let _ = chars.next();
                    text.push('}');
                }
                Some(other) if other.is_ascii_alphabetic() => {
                    // An unbraced override is not valid ASS text; keep the
                    // payload literal and flag it rather than dropping text.
                    unsupported = true;
                    text.push(character);
                }
                _ => text.push(character),
            },
            _ => text.push(character),
        }
        if text.len() > MAX_ASS_TEXT_BYTES {
            truncated = true;
            text.truncate(MAX_ASS_TEXT_BYTES);
            break;
        }
    }
    StrippedText {
        text,
        overrides: if overrides.is_empty() {
            None
        } else {
            Some(overrides)
        },
        unsupported,
        truncated,
    }
}

fn apply_overrides(
    style: &mut CueStyle,
    overrides: &[String],
    play_res_x: f64,
    play_res_y: f64,
    warnings: &mut Vec<AssWarning>,
    suppressed: &mut usize,
    line: usize,
) {
    let mut unsupported = false;
    let base = style.clone();
    for block in overrides {
        for command in commands(block).into_iter().skip(1) {
            let command = command.trim();
            if command.is_empty() {
                continue;
            }
            let (tag, argument) = split_tag(command);
            let argument = argument.trim();
            match tag {
                "b" => style.bold = override_bool(argument, style.bold, base.bold),
                "i" => style.italic = override_bool(argument, style.italic, base.italic),
                "u" => style.underline = override_bool(argument, style.underline, base.underline),
                "s" => style.strikeout = override_bool(argument, style.strikeout, base.strikeout),
                "fs" => {
                    if let Some(value) = number(argument)
                        .filter(|v| *v > 0.0)
                        .and_then(|v| ratio(Some(v), play_res_y))
                        .filter(|v| *v > 0.0)
                    {
                        style.font_size_ratio = value;
                    } else {
                        unsupported = true;
                    }
                }
                "an" => {
                    if let Some(value) = number(argument) {
                        style.alignment = alignment(value);
                    } else {
                        unsupported = true;
                    }
                }
                "c" | "1c" => {
                    if let Some(value) = color(argument) {
                        style.primary_color = keep_alpha(value, style.primary_color);
                    } else {
                        unsupported = true;
                    }
                }
                "3c" => {
                    if let Some(value) = color(argument) {
                        style.outline_color = keep_alpha(value, style.outline_color);
                    } else {
                        unsupported = true;
                    }
                }
                "4c" => {
                    if let Some(value) = color(argument) {
                        style.background_color = keep_alpha(value, style.background_color);
                    } else {
                        unsupported = true;
                    }
                }
                // Transforms, karaoke and animation change layout over time.
                // Importing them as static text would invent rendering.
                "pos" | "move" | "fad" | "fade" | "t" | "clip" | "iclip" | "org" | "k" | "K"
                | "kf" | "ko" | "kt" | "p" | "pbo" | "alpha" | "a" | "r" | "q" | "fe" | "blur"
                | "bord" | "shad" | "xbord" | "ybord" | "xshad" | "yshad" | "fscx" | "fscy"
                | "frx" | "fry" | "frz" | "fax" | "fay" => {
                    unsupported = true;
                }
                _ => unsupported = true,
            }
        }
    }
    // Margins can be overridden with `\q`/drawing tags only in unsupported
    // forms; the parsed style keeps the base margins.
    let _ = play_res_x;
    if unsupported {
        warn(
            warnings,
            suppressed,
            line,
            AssWarningKind::UnsupportedOverride,
        );
    }
}

fn split_tag(command: &str) -> (&str, &str) {
    // Colour tags can start with a component digit (`\1c`, `\3c`, `\4c`).
    let bytes = command.as_bytes();
    if bytes.first().is_some_and(|b| b.is_ascii_digit())
        && bytes.get(1).is_some_and(|b| b.is_ascii_alphabetic())
    {
        return command.split_at(2);
    }
    let end = command
        .char_indices()
        .find(|(_, c)| c.is_ascii_digit() || *c == '(' || *c == '&' || *c == '-')
        .map_or(command.len(), |(index, _)| index);
    command.split_at(end)
}

/// `\c` sets the colour channels only; alpha stays with the style.
fn keep_alpha(value: u32, current: [u8; 4]) -> [u8; 4] {
    let [red, green, blue, _] = rgba(Some(value), current);
    [red, green, blue, current[3]]
}

/// Split an override block on `\` outside parentheses, so a `\t(0,1,\fs50)`
/// transform stays one command. The first piece precedes the first `\`.
fn commands(block: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, character) in block.char_indices() {
        match character {
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            '\\' if depth == 0 => {
                pieces.push(&block[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    pieces.push(&block[start..]);
    pieces
}

/// Last `\p<n>` in a block: `Some(true)` starts drawing mode, `Some(false)` ends it.
fn drawing_mode(block: &str) -> Option<bool> {
    commands(block)
        .into_iter()
        .skip(1)
        .filter_map(|command| {
            let digits = command.trim().strip_prefix('p')?;
            (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                .then(|| !digits.trim_start_matches('0').is_empty())
        })
        .next_back()
}

/// A bare `\b`/`\i`/`\u`/`\s` restores the style's own value.
fn override_bool(argument: &str, current: bool, base: bool) -> bool {
    match argument.trim() {
        "" => base,
        value => value.parse::<i64>().map(|v| v != 0).unwrap_or(current),
    }
}
