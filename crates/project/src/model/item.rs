//! Item identities, typed item parameters and item timing.
use super::project::{CaptionPresetId, text};
use super::settings::Color;
use super::track::TrackKind;
use crate::visual::DEFAULT_WEIGHT;
use crate::{Error, Result};
use captions::{CueTrack, NfcText};
use media::AssetId;
use serde::{Deserialize, Serialize};
use time::{SourceRange, Tick};

id!(ItemId);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VideoParams {
    pub source_audio_enabled: bool,
    pub volume: f32,
    pub muted: bool,
    /// Source-normalized region shown in place of the whole frame; absent keeps the full frame.
    #[serde(default)]
    pub crop: Option<crate::Crop>,
    /// How the cropped source is placed on the canvas, evaluated at render time.
    #[serde(default)]
    pub fit: crate::FitMode,
    #[serde(default)]
    pub fades: crate::Fades,
    /// Non-destructive cleanup of the item's source audio; the source media is never changed.
    #[serde(default)]
    pub cleanup: crate::AudioCleanup,
    /// Fill behind the clip wherever the clip does not cover the canvas; absent is none.
    #[serde(default)]
    pub background: ClipBackground,
    /// Per-item switch for the edge-preserving noise-reduction pass; off leaves the picture untouched.
    #[serde(default)]
    pub reduce_noise: bool,
}
impl VideoParams {
    /// A plain clip: its own audio off, unity volume, not muted, the full frame, default
    /// fit, no fades, no cleanup, no background and no noise reduction. `Default` would
    /// need a neutral `source_audio_enabled`, which has none: a clip with its audio on and
    /// one without are both ordinary, so sites that want audio set it as a field update.
    pub fn plain() -> Self {
        Self {
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            crop: None,
            fit: crate::FitMode::default(),
            fades: crate::Fades::default(),
            cleanup: crate::AudioCleanup::default(),
            background: ClipBackground::None,
            reduce_noise: false,
        }
    }
}
/// What fills the canvas behind a visual clip. Exclusive: one kind per clip.
/// The blur reads the clip's own decoded frame, so it never feeds back.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClipBackground {
    #[default]
    None,
    Solid {
        color: Color,
    },
    Blur {
        intensity: u32,
    },
}
impl ClipBackground {
    /// Intensity a clip starts at when Blur is chosen; a preset replaces it.
    pub const DEFAULT_BLUR_INTENSITY: u32 = 10;
    /// Light, Medium and Heavy presets, in reference order.
    pub const BLUR_PRESETS: [u32; 3] = [100, 200, 500];
    /// Largest accepted blur intensity: the Heavy preset.
    pub const MAX_BLUR_INTENSITY: u32 = 500;
    /// Whether a blur intensity lies within the accepted range.
    pub const fn valid(self) -> bool {
        match self {
            Self::None | Self::Solid { .. } => true,
            Self::Blur { intensity } => intensity <= Self::MAX_BLUR_INTENSITY,
        }
    }
}
/// Remote-library provenance carried with an audio item. URLs are canonical and
/// bounded; no token, cache path or local absolute path belongs here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibrarySound {
    /// Provider identity; Freesound sound identifiers are positive.
    pub id: u64,
    pub name: NfcText,
    pub author: NfcText,
    pub licence: NfcText,
    /// Human-readable attribution shown in the panel and written into exports.
    pub attribution: NfcText,
    /// Public provider page; the item's offline media is a cached preview.
    pub source_url: String,
    /// Exact preview URL the cached bytes were fetched from. Changing it
    /// invalidates the content-addressed cache entry.
    pub audio_url: String,
    pub downloads: u64,
    /// Provider-reported duration for display before the cached file is probed.
    pub duration_ms: u64,
}
/// Music provenance carried with an audio item that came from the open music
/// source. The identifier is the source's own opaque id, so a track keeps its
/// licence and attribution wherever the project is opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LibraryMusic {
    /// Source identifier (the aggregator's UUID).
    pub id: NfcText,
    pub title: NfcText,
    pub author: NfcText,
    /// Short licence name shown beside the author, e.g. `CC BY 4.0`.
    pub licence: NfcText,
    /// Licence deed the track is used under.
    pub licence_url: String,
    /// Human-readable attribution written into exports.
    pub attribution: NfcText,
    /// Source page for the track; the item's offline media is a cached file.
    pub source_url: String,
    /// Exact audio URL the cached bytes were fetched from.
    pub audio_url: String,
    /// Source-reported duration for display before the cached file is probed.
    pub duration_ms: u64,
}
impl LibraryMusic {
    /// Admit provenance before saving or inserting it into a project.
    ///
    /// IDs contain 1..=128 ASCII letters/digits or `-_.:` bytes. NFC title, author
    /// and attribution contain 1..=4096 UTF-8 bytes without controls. URLs are HTTPS, at most 8192
    /// bytes, with a valid authority and no credentials. Duration is 1..=86400000 ms.
    /// Licence name and deed must identify the same admitted [`MusicLicence`].
    /// Returns typed `Invalid`, `Unsupported` or `TooLarge` with static diagnostics;
    /// this validation performs no network or filesystem IO.
    pub fn validate(&self) -> Result<()> {
        if self.id.as_str().len() > 128 {
            return Err(Error::TooLarge("library music id"));
        }
        if self.id.as_str().is_empty()
            || !self
                .id
                .as_str()
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
        {
            return Err(Error::Invalid("library music id"));
        }
        for value in [&self.title, &self.author, &self.attribution] {
            if value.as_str().len() > 4096 {
                return Err(Error::TooLarge("library music text"));
            }
            if value.as_str().trim().is_empty() || value.as_str().chars().any(char::is_control) {
                return Err(Error::Invalid("library music attribution"));
            }
        }
        MusicLicence::parse(self.licence.as_str(), &self.licence_url)?;
        music_url(&self.source_url)?;
        music_url(&self.audio_url)?;
        if self.duration_ms == 0 || self.duration_ms > 86_400_000 {
            return Err(Error::Invalid("library music duration"));
        }
        Ok(())
    }
    /// Attribution record written into the container comment.
    pub fn attribution_record(&self) -> String {
        format!(
            "{} — {} — {} — {}",
            self.attribution.as_str(),
            self.licence.as_str(),
            self.licence_url,
            self.source_url
        )
    }
}

/// An admitted music dedication/licence identity, independent of persisted fields.
/// Construct through [`Self::parse`]; no network lookup or inference is performed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicLicence {
    name: &'static str,
    deed: &'static str,
}
impl MusicLicence {
    /// Match an exact CC0 1.0 or global CC BY 1.0/2.0/2.5/3.0/4.0 name and deed.
    /// Only omission of the final deed slash is normalized. Ported, versionless,
    /// unknown and restrictive licences return `Unsupported`; a contradictory or
    /// noncanonical deed returns `Invalid`. Names over 4096 UTF-8 bytes or URLs
    /// over 8192 bytes return `TooLarge`. Diagnostics never include supplied text.
    pub fn parse(display: &str, deed: &str) -> Result<Self> {
        const IDENTITIES: [(&str, &str); 6] = [
            (
                "CC0 1.0",
                "https://creativecommons.org/publicdomain/zero/1.0/",
            ),
            ("CC BY 1.0", "https://creativecommons.org/licenses/by/1.0/"),
            ("CC BY 2.0", "https://creativecommons.org/licenses/by/2.0/"),
            ("CC BY 2.5", "https://creativecommons.org/licenses/by/2.5/"),
            ("CC BY 3.0", "https://creativecommons.org/licenses/by/3.0/"),
            ("CC BY 4.0", "https://creativecommons.org/licenses/by/4.0/"),
        ];
        if display.len() > 4096 || deed.len() > 8192 {
            return Err(Error::TooLarge("music licence identity"));
        }
        let (name, canonical) = IDENTITIES
            .into_iter()
            .find(|(name, _)| *name == display)
            .ok_or(Error::Unsupported("music licence identity"))?;
        if deed != canonical && Some(deed) != canonical.strip_suffix('/') {
            return Err(Error::Invalid("music licence deed"));
        }
        Ok(Self {
            name,
            deed: canonical,
        })
    }

    /// Canonical display name belonging to the admitted deed.
    pub fn display_name(&self) -> &'static str {
        self.name
    }

    /// Canonical HTTPS deed URL, including its final slash.
    pub fn deed_url(&self) -> &'static str {
        self.deed
    }
}

fn music_url(value: &str) -> Result<()> {
    if value.len() > 8192 {
        return Err(Error::TooLarge("library music URL"));
    }
    let invalid = || Error::Invalid("library music URL");
    let rest = value.strip_prefix("https://").ok_or_else(invalid)?;
    if value
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '\\' | '"' | '<' | '>' | '`'))
    {
        return Err(invalid());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let port = if let Some(ipv6) = authority.strip_prefix('[') {
        let (host, suffix) = ipv6.split_once(']').ok_or_else(invalid)?;
        host.parse::<std::net::Ipv6Addr>().map_err(|_| invalid())?;
        if suffix.is_empty() {
            None
        } else {
            Some(suffix.strip_prefix(':').ok_or_else(invalid)?)
        }
    } else {
        let (host, port) = authority
            .split_once(':')
            .map_or((authority, None), |(host, port)| (host, Some(port)));
        if host.is_empty()
            || host.len() > 253
            || !host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && label
                        .as_bytes()
                        .first()
                        .is_some_and(u8::is_ascii_alphanumeric)
                    && label
                        .as_bytes()
                        .last()
                        .is_some_and(u8::is_ascii_alphanumeric)
            })
        {
            return Err(invalid());
        }
        if host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            host.parse::<std::net::Ipv4Addr>().map_err(|_| invalid())?;
        }
        port
    };
    if let Some(port) = port
        && (port.is_empty()
            || !port.bytes().all(|b| b.is_ascii_digit())
            || port.parse::<u16>().is_err())
    {
        return Err(invalid());
    }
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%'
            && !(bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
                && bytes.next().is_some_and(|b| b.is_ascii_hexdigit()))
        {
            return Err(invalid());
        }
    }
    Ok(())
}
impl LibrarySound {
    pub(super) fn validate(&self) -> Result<()> {
        if self.id == 0 {
            return Err(Error::Invalid("library sound id"));
        }
        text(&self.name)?;
        text(&self.author)?;
        text(&self.licence)?;
        text(&self.attribution)?;
        if self.name.as_str().is_empty()
            || self.author.as_str().is_empty()
            || self.licence.as_str().is_empty()
            || self.attribution.as_str().is_empty()
        {
            return Err(Error::Invalid("library sound attribution"));
        }
        https(&self.source_url)?;
        https(&self.audio_url)?;
        if self.duration_ms > 86_400_000 {
            return Err(Error::Invalid("library sound duration"));
        }
        Ok(())
    }
}
fn https(value: &str) -> Result<()> {
    if value.len() > 8192
        || !value.starts_with("https://")
        || value
            .bytes()
            .any(|b| b.is_ascii_control() || b == b' ' || b == b'\\' || b == b'"')
    {
        return Err(Error::Invalid("library sound URL"));
    }
    Ok(())
}
/// Audio items either reference a project asset or a cached provider preview.
/// Ordinary uploads keep the default so no content field is duplicated.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "sourceType", rename_all = "snake_case", deny_unknown_fields)]
pub enum AudioSource {
    #[default]
    Upload,
    Library {
        sound: LibrarySound,
    },
    Music {
        music: LibraryMusic,
    },
}
/// `Default` is the plain clip: unity volume (0 dB), not muted, an uploaded source,
/// no fades and no cleanup. Every field has a neutral value, so no `plain()` is needed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioParams {
    pub volume: f32,
    pub muted: bool,
    #[serde(default)]
    pub source: AudioSource,
    #[serde(default)]
    pub fades: crate::Fades,
    /// Non-destructive cleanup of the item's source audio; the source media is never changed.
    #[serde(default)]
    pub cleanup: crate::AudioCleanup,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextBackground {
    pub color: Color,
    /// Horizontal padding in project canvas pixels; scaled with the output like the font size.
    pub padding_x: f32,
    /// Vertical padding in project canvas pixels.
    pub padding_y: f32,
    /// Shift of the box from the text bounds in project canvas pixels (y down).
    pub offset_x: f32,
    pub offset_y: f32,
    pub radius: f32,
}
impl Default for TextBackground {
    fn default() -> Self {
        Self {
            color: Color([0, 0, 0, 160]),
            padding_x: 12.0,
            padding_y: 12.0,
            offset_x: 0.0,
            offset_y: 0.0,
            radius: 4.0,
        }
    }
}
impl TextBackground {
    /// Largest padding or radius in project canvas pixels.
    pub const MAX_EXTENT: f32 = 128.0;
    /// Largest box offset magnitude in project canvas pixels.
    pub const MAX_OFFSET: f32 = 1024.0;
    /// Whether every geometry field is finite and within its documented range.
    pub fn valid(&self) -> bool {
        let extent = |v: f32| v.is_finite() && (0.0..=Self::MAX_EXTENT).contains(&v);
        let offset = |v: f32| v.is_finite() && v.abs() <= Self::MAX_OFFSET;
        extent(self.padding_x)
            && extent(self.padding_y)
            && extent(self.radius)
            && offset(self.offset_x)
            && offset(self.offset_y)
    }
}
/// An outer stroke drawn around the glyph fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextStroke {
    pub color: Color,
    /// Stroke width in project canvas pixels; zero draws nothing.
    pub width: f32,
}
impl TextStroke {
    /// Largest accepted stroke width in project canvas pixels.
    pub const MAX_WIDTH: f32 = 64.0;
    /// Whether the width is finite and within the documented range.
    pub fn valid(&self) -> bool {
        self.width.is_finite() && (0.0..=Self::MAX_WIDTH).contains(&self.width)
    }
}
impl Default for TextStroke {
    fn default() -> Self {
        Self {
            color: Color([0, 0, 0, 255]),
            width: 4.0,
        }
    }
}
/// A soft, offset copy of the glyph coverage painted behind the fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextShadow {
    pub color: Color,
    /// Offset from the glyph coverage in project canvas pixels (y down).
    pub offset_x: f32,
    pub offset_y: f32,
    /// Blur radius in project canvas pixels; zero keeps a hard copy.
    pub blur: f32,
}
impl TextShadow {
    /// Largest offset magnitude in project canvas pixels.
    pub const MAX_OFFSET: f32 = 1024.0;
    /// Largest blur radius in project canvas pixels.
    pub const MAX_BLUR: f32 = 64.0;
    /// Whether every field is finite and within its documented range.
    pub fn valid(&self) -> bool {
        let offset = |v: f32| v.is_finite() && v.abs() <= Self::MAX_OFFSET;
        offset(self.offset_x)
            && offset(self.offset_y)
            && self.blur.is_finite()
            && (0.0..=Self::MAX_BLUR).contains(&self.blur)
    }
}
impl Default for TextShadow {
    fn default() -> Self {
        Self {
            color: Color([0, 0, 0, 160]),
            offset_x: 4.0,
            offset_y: 4.0,
            blur: 4.0,
        }
    }
}
/// Horizontal alignment of lines inside the text box.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}
/// Line drawn across the text in its fill colour.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TextDecoration {
    #[default]
    None,
    Underline,
    LineThrough,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextParams {
    pub content: NfcText,
    /// Font family name, resolved by the text crate; an unavailable family renders in the
    /// bundled default.
    pub font: NfcText,
    pub font_size: f32,
    /// CSS weight 100..=900 (400 regular, 700 bold).
    pub weight: u16,
    pub italic: bool,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub decoration: TextDecoration,
    /// Extra advance after every grapheme cluster, in project canvas pixels (scaled with
    /// the output like the font size); negative tightens.
    pub letter_spacing: f32,
    /// Baseline distance in project canvas pixels; zero uses the font's natural height.
    pub line_height: f32,
    pub color: Color,
    #[serde(default)]
    pub stroke: Option<TextStroke>,
    #[serde(default)]
    pub shadow: Option<TextShadow>,
    /// Paint a soft halo in the fill colour behind the glyphs.
    #[serde(default)]
    pub glow: bool,
    pub background: Option<TextBackground>,
}
impl Default for TextParams {
    fn default() -> Self {
        Self::new("")
    }
}
impl TextParams {
    /// Baseline title style shared by insertion and fixtures.
    pub fn new(content: impl Into<NfcText>) -> Self {
        Self {
            content: content.into(),
            font: "Inter".into(),
            font_size: 48.0,
            weight: DEFAULT_WEIGHT,
            italic: false,
            align: TextAlign::default(),
            decoration: TextDecoration::default(),
            letter_spacing: 0.0,
            line_height: 0.0,
            color: Color([255; 4]),
            stroke: None,
            shadow: None,
            glow: false,
            background: None,
        }
    }
}
/// Reusable text appearance copied from one item into ordinary text parameters.
/// Content is never part of a preset, so applying one keeps every item's words.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextStyle {
    pub font: NfcText,
    pub font_size: f32,
    pub weight: u16,
    pub italic: bool,
    pub align: TextAlign,
    pub decoration: TextDecoration,
    pub letter_spacing: f32,
    pub line_height: f32,
    pub color: Color,
    pub stroke: Option<TextStroke>,
    pub shadow: Option<TextShadow>,
    pub glow: bool,
    pub background: Option<TextBackground>,
}
impl TextStyle {
    /// The style of `params`, leaving its content behind.
    pub fn from_params(params: &TextParams) -> Self {
        Self {
            font: params.font.clone(),
            font_size: params.font_size,
            weight: params.weight,
            italic: params.italic,
            align: params.align,
            decoration: params.decoration,
            letter_spacing: params.letter_spacing,
            line_height: params.line_height,
            color: params.color,
            stroke: params.stroke.clone(),
            shadow: params.shadow.clone(),
            glow: params.glow,
            background: params.background.clone(),
        }
    }
    /// Copy this style over `params`, keeping its content.
    pub fn apply_to(&self, params: &mut TextParams) {
        params.font = self.font.clone();
        params.font_size = self.font_size;
        params.weight = self.weight;
        params.italic = self.italic;
        params.align = self.align;
        params.decoration = self.decoration;
        params.letter_spacing = self.letter_spacing;
        params.line_height = self.line_height;
        params.color = self.color;
        params.stroke = self.stroke.clone();
        params.shadow = self.shadow.clone();
        params.glow = self.glow;
        params.background = self.background.clone();
    }
    /// Whether every stored field satisfies the text-item bounds.
    pub fn valid(&self) -> bool {
        crate::visual::valid(
            self.font.as_str(),
            self.weight,
            self.letter_spacing,
            self.line_height,
        ) && self.font_size.is_finite()
            && (1.0..=4096.0).contains(&self.font_size)
            && self.stroke.as_ref().is_none_or(TextStroke::valid)
            && self.shadow.as_ref().is_none_or(TextShadow::valid)
            && self.background.as_ref().is_none_or(TextBackground::valid)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionParams {
    /// Explicit source-word emphasis; new and imported caption items start disabled.
    pub word_highlight: bool,
    /// Live saved-preset selection; absent keeps imported cue styles and project settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<CaptionPresetId>,
    /// Sparse patch over the selected preset; requires a selection.
    #[serde(default, skip_serializing_if = "captions::CaptionOverrides::is_empty")]
    pub overrides: captions::CaptionOverrides,
    /// Keyword emphasis per cue ID: ascending verified word ids (indices into the cue's
    /// `CueWordTiming::Words` records) painted by the word-highlight overlay. Read only through
    /// `captions::resolve_selection`; ids that no longer match the cue's words are ignored.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub emphasis: std::collections::BTreeMap<captions::NfcText, Vec<u32>>,
    pub cues: CueTrack,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OverlayParams {
    pub color: Color,
}
/// A bundled sticker: its `provider:value` id and the intrinsic pixel size captured at insert.
/// Stickers take transform, blend and effects but no masks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StickerParams {
    pub sticker: String,
    pub width: u32,
    pub height: u32,
}
impl StickerParams {
    /// Longest accepted sticker id in bytes.
    pub const MAX_ID_BYTES: usize = 128;
    /// Largest accepted intrinsic side in pixels.
    pub const MAX_SIDE: u32 = 16384;
    pub fn new(sticker: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            sticker: sticker.into(),
            width,
            height,
        }
    }
    /// The provider half of the id.
    pub fn provider(&self) -> &str {
        self.sticker
            .split_once(':')
            .map_or("", |(provider, _)| provider)
    }
    /// The value half of the id.
    pub fn value(&self) -> &str {
        self.sticker.split_once(':').map_or("", |(_, value)| value)
    }
    /// `provider:value` of ASCII letters, digits, `-`, `_` and `.`, with an intrinsic size in range.
    pub fn validate(&self) -> Result<()> {
        let part = |text: &str| {
            !text.is_empty()
                && text
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        if self.sticker.len() > Self::MAX_ID_BYTES
            || !part(self.provider())
            || !part(self.value())
            || self.sticker.matches(':').count() != 1
            || !(1..=Self::MAX_SIDE).contains(&self.width)
            || !(1..=Self::MAX_SIDE).contains(&self.height)
        {
            return Err(Error::Invalid("sticker parameters"));
        }
        Ok(())
    }
}
/// Generated geometry in a 512-pixel author square. Saved appearance is explicit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "definitionId",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum GraphicParams {
    Rectangle {
        fill: Color,
        stroke: Color,
        #[serde(rename = "strokeWidth")]
        stroke_width: f64,
        #[serde(rename = "strokeAlign")]
        stroke_align: crate::StrokeAlign,
        #[serde(rename = "cornerRadius")]
        corner_radius: f64,
    },
    Ellipse {
        fill: Color,
        stroke: Color,
        #[serde(rename = "strokeWidth")]
        stroke_width: f64,
        #[serde(rename = "strokeAlign")]
        stroke_align: crate::StrokeAlign,
    },
    Polygon {
        fill: Color,
        stroke: Color,
        #[serde(rename = "strokeWidth")]
        stroke_width: f64,
        #[serde(rename = "strokeAlign")]
        stroke_align: crate::StrokeAlign,
        sides: f64,
        #[serde(rename = "cornerRadius")]
        corner_radius: f64,
    },
    Star {
        fill: Color,
        stroke: Color,
        #[serde(rename = "strokeWidth")]
        stroke_width: f64,
        #[serde(rename = "strokeAlign")]
        stroke_align: crate::StrokeAlign,
        points: f64,
        depth: f64,
    },
}
impl GraphicParams {
    pub fn rectangle() -> Self {
        Self::Rectangle {
            fill: Color([255; 4]),
            stroke: Color([0, 0, 0, 255]),
            stroke_width: 0.,
            stroke_align: crate::StrokeAlign::Center,
            corner_radius: 0.,
        }
    }
    pub fn ellipse() -> Self {
        Self::Ellipse {
            fill: Color([255; 4]),
            stroke: Color([0, 0, 0, 255]),
            stroke_width: 0.,
            stroke_align: crate::StrokeAlign::Center,
        }
    }
    pub fn polygon() -> Self {
        Self::Polygon {
            fill: Color([255; 4]),
            stroke: Color([0, 0, 0, 255]),
            stroke_width: 0.,
            stroke_align: crate::StrokeAlign::Center,
            sides: 5.,
            corner_radius: 0.,
        }
    }
    pub fn star() -> Self {
        Self::Star {
            fill: Color([255; 4]),
            stroke: Color([0, 0, 0, 255]),
            stroke_width: 0.,
            stroke_align: crate::StrokeAlign::Center,
            points: 5.,
            depth: 45.,
        }
    }
    /// Stored fallbacks; present animation parameters remain authoritative, even without keys.
    pub fn appearance(&self) -> (Color, Color, f64, crate::StrokeAlign, f64) {
        match *self {
            Self::Rectangle {
                fill,
                stroke,
                stroke_width,
                stroke_align,
                corner_radius,
            } => (fill, stroke, stroke_width, stroke_align, corner_radius),
            Self::Polygon {
                fill,
                stroke,
                stroke_width,
                stroke_align,
                corner_radius,
                ..
            } => (fill, stroke, stroke_width, stroke_align, corner_radius),
            Self::Ellipse {
                fill,
                stroke,
                stroke_width,
                stroke_align,
            }
            | Self::Star {
                fill,
                stroke,
                stroke_width,
                stroke_align,
                ..
            } => (fill, stroke, stroke_width, stroke_align, 0.),
        }
    }
    /// Stored radial geometry: polygon sides, star points, star depth percent.
    /// Values outside a definition's own fields are zero and unused.
    pub fn radial(&self) -> (f64, f64, f64) {
        match *self {
            Self::Polygon { sides, .. } => (sides, 0., 0.),
            Self::Star { points, depth, .. } => (0., points, depth),
            _ => (0., 0., 0.),
        }
    }
    pub(crate) fn validate(&self, animations: &crate::Animations) -> Result<()> {
        let (_, _, width, _, radius) = self.appearance();
        let radial_valid = match *self {
            Self::Polygon { sides, .. } => sides.is_finite() && (3. ..=12.).contains(&sides),
            Self::Star { points, depth, .. } => {
                points.is_finite()
                    && (3. ..=12.).contains(&points)
                    && depth.is_finite()
                    && (1. ..=99.).contains(&depth)
            }
            _ => true,
        };
        if !width.is_finite()
            || !(0. ..=64.).contains(&width)
            || !radius.is_finite()
            || !(0. ..=50.).contains(&radius)
            || !radial_valid
        {
            return Err(Error::Invalid("graphic appearance"));
        }
        let bounded = |channel: &crate::Channel, range: std::ops::RangeInclusive<f64>| {
            let valid = |value: &crate::ParamValue| {
                matches!(value, crate::ParamValue::Scalar(value)
                    if value.is_finite() && range.contains(value))
            };
            valid(channel.base()) && channel.keys().iter().all(|key| valid(&key.value))
        };
        for (path, param) in animations.iter() {
            let Some(name) = path.as_str().strip_prefix("params.") else {
                continue;
            };
            match (name, param) {
                ("fill" | "stroke", crate::Param::Color(_)) => {}
                ("strokeWidth", crate::Param::Value(channel)) => {
                    if !bounded(channel, 0. ..=64.) {
                        return Err(Error::Invalid("graphic animation"));
                    }
                }
                ("cornerRadius", crate::Param::Value(channel)) => {
                    if !matches!(self, Self::Rectangle { .. } | Self::Polygon { .. })
                        || !bounded(channel, 0. ..=50.)
                    {
                        return Err(Error::Invalid("graphic animation"));
                    }
                }
                ("sides", crate::Param::Value(channel)) => {
                    if !matches!(self, Self::Polygon { .. }) || !bounded(channel, 3. ..=12.) {
                        return Err(Error::Invalid("graphic animation"));
                    }
                }
                ("points", crate::Param::Value(channel)) => {
                    if !matches!(self, Self::Star { .. }) || !bounded(channel, 3. ..=12.) {
                        return Err(Error::Invalid("graphic animation"));
                    }
                }
                ("depth", crate::Param::Value(channel)) => {
                    if !matches!(self, Self::Star { .. }) || !bounded(channel, 1. ..=99.) {
                        return Err(Error::Invalid("graphic animation"));
                    }
                }
                ("strokeAlign", crate::Param::Value(channel)) => {
                    let valid = |value: &crate::ParamValue| {
                        matches!(value, crate::ParamValue::Text(value)
                            if matches!(value.as_str(), "inside" | "center" | "outside"))
                    };
                    if !valid(channel.base())
                        || channel.keys().iter().any(|key| !valid(&key.value))
                        || channel.extrapolation()
                            != (crate::Extrapolation::Hold, crate::Extrapolation::Hold)
                        || channel
                            .keys()
                            .iter()
                            .any(|key| key.segment != crate::Segment::Step)
                    {
                        return Err(Error::Invalid("graphic animation"));
                    }
                }
                _ => return Err(Error::Invalid("graphic animation path or type")),
            }
        }
        Ok(())
    }
}
/// Typed parameters determine item kind; no unvalidated arbitrary parameter bag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "kind",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ItemParams {
    Video(VideoParams),
    Audio(AudioParams),
    Text(TextParams),
    Caption(CaptionParams),
    Overlay(OverlayParams),
    Graphic(GraphicParams),
    Sticker(StickerParams),
}
impl ItemParams {
    pub fn kind(&self) -> TrackKind {
        match self {
            Self::Video(_) => TrackKind::Video,
            Self::Audio(_) => TrackKind::Audio,
            Self::Text(_) => TrackKind::Text,
            Self::Caption(_) => TrackKind::Text,
            Self::Overlay(_) | Self::Graphic(_) => TrackKind::Video,
            Self::Sticker(_) => TrackKind::Sticker,
        }
    }
    /// Visual kinds that carry a placed raster layer a reflection can mirror.
    /// Captions own their saved cue layout and audio has no picture.
    pub fn supports_flip(&self) -> bool {
        self.has_placed_box()
    }
    /// Visual kinds whose placed box alignment and even spacing may translate.
    /// Captions own their saved cue layout and audio has no picture.
    pub fn supports_arrange(&self) -> bool {
        self.has_placed_box()
    }
    /// Visual kinds whose placed box the In, Out and Combo animations may move.
    pub fn supports_animation(&self) -> bool {
        self.has_placed_box()
    }
    fn has_placed_box(&self) -> bool {
        matches!(
            self,
            Self::Video(_) | Self::Text(_) | Self::Overlay(_) | Self::Graphic(_) | Self::Sticker(_)
        )
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Item {
    #[serde(rename = "blendMode")]
    pub blend_mode: crate::BlendMode,
    pub masks: Vec<crate::Mask>,
    pub effects: Vec<crate::EffectInstance>,
    pub animations: crate::Animations,
    pub id: ItemId,
    /// Visual item hidden in the preview and export while its audio stays audible.
    #[serde(default)]
    pub hidden: bool,
    pub source_asset: Option<AssetId>,
    /// Source ticks for media/captions; local content ticks for generated items.
    pub source_range: SourceRange,
    /// Output timeline ticks. Playback uses the retained retime slice when present; see [`Item::duration`].
    pub start: Tick,
    pub retime: crate::Retime,
    pub content: ItemParams,
}
impl Item {
    pub fn end(&self) -> Result<Tick> {
        self.start
            .checked_add(self.duration()?)
            .ok_or(Error::Invalid("item end overflow"))
    }

    /// Native ASR words of a caption item, each clipped to its cue and the item's
    /// source window, with source and output ranges. Other item kinds carry none.
    ///
    /// # Errors
    ///
    /// Returns an error when the retained retime mapping or a range is unrepresentable.
    pub fn word_spans(&self) -> Result<Vec<WordSpan>> {
        let ItemParams::Caption(params) = &self.content else {
            return Ok(Vec::new());
        };
        let mut spans = Vec::new();
        for cue in &params.cues.cues {
            for (index, word) in cue.word_ranges().iter().enumerate() {
                let start = word
                    .start()
                    .max(self.source_range.start())
                    .max(cue.range.start());
                let end = word.end().min(self.source_range.end()).min(cue.range.end());
                if start >= end {
                    continue;
                }
                let source = SourceRange::new(start, end)?;
                let output =
                    SourceRange::new(self.output_time_of(start)?, self.output_time_of(end)?)?;
                if output.start() >= output.end() {
                    continue;
                }
                spans.push(WordSpan {
                    item: self.id,
                    cue: cue.id.clone(),
                    index,
                    source,
                    output,
                });
            }
        }
        Ok(spans)
    }
}

/// One native ASR word retained by a caption item, in source and output time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordSpan {
    pub item: ItemId,
    pub cue: NfcText,
    pub index: usize,
    pub source: SourceRange,
    pub output: SourceRange,
}

/// Clip/master dB range and hundredth-dB editing grid. Curves interpolate continuously.
pub const VOLUME_DB_RANGE: std::ops::RangeInclusive<f32> = -60.0..=20.0;

pub fn valid_volume(value: f32) -> bool {
    value.is_finite()
        && VOLUME_DB_RANGE.contains(&value)
        && (value * 100.0 - (value * 100.0).round()).abs() < 0.001
}

/// Safe captured-state diagnostics. No cue text, arbitrary cue ID or source path is exposed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionWordWarning {
    pub item: ItemId,
    /// Zero-based retained cue position, never the user-authored cue ID.
    pub cue: usize,
    pub language: captions::LanguageTag,
    pub reason: captions::WordHighlightReason,
}
/// At most 64 item/language/reason summaries; additional unrepresented cues are counted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionWordWarnings {
    #[schemars(length(max = 64))]
    pub warnings: Vec<CaptionWordWarning>,
    pub additional: usize,
}
