use crate::{Error, Result};
use captions::{CueTrack, NfcText};
use media::{AssetId, SourceAsset, StreamKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::{FrameRate, SourceRange, Tick};

macro_rules! id {
    ($($name:ident),+) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(transparent)]
        pub struct $name(pub u64);
    )+};
}
id!(TrackId, ItemId, BookmarkId, SceneId);

/// Decimal-string identity, including on disk, so control clients retain every bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, schemars::JsonSchema)]
#[schemars(with = "String")]
pub struct ProjectId(pub u64);
impl Serialize for ProjectId {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for ProjectId {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        let id = value
            .parse::<u64>()
            .map_err(|_| serde::de::Error::custom("invalid project identity"))?;
        if id.to_string() != value {
            return Err(serde::de::Error::custom("noncanonical project identity"));
        }
        Ok(Self(id))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct Color(pub [u8; 4]);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanvasSize {
    pub width: u32,
    pub height: u32,
}
impl CanvasSize {
    /// Whether both axes are positive and the frame stays inside the canvas area bound.
    pub const fn is_valid(self) -> bool {
        self.width > 0 && self.height > 0 && (self.width as u64) * (self.height as u64) <= 1 << 26
    }
    /// The nearest even width and height, ties up: an odd axis grows by one so
    /// encoders can open the frame. `None` when an odd maximum axis overflows.
    pub const fn rounded_even(self) -> Option<Self> {
        let width = if self.width.is_multiple_of(2) {
            self.width
        } else {
            match self.width.checked_add(1) {
                Some(width) => width,
                None => return None,
            }
        };
        let height = if self.height.is_multiple_of(2) {
            self.height
        } else {
            match self.height.checked_add(1) {
                Some(height) => height,
                None => return None,
            }
        };
        Some(Self { width, height })
    }
}
/// One fixed aspect preset offered by the canvas settings. `Original` resolves
/// to the retained first visual media size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AspectPreset {
    Original,
    #[serde(rename = "16:9")]
    Landscape,
    #[serde(rename = "4:3")]
    Standard,
    #[serde(rename = "2:1")]
    Wide,
    #[serde(rename = "9:16")]
    Portrait,
    #[serde(rename = "1:1")]
    Square,
    #[serde(rename = "3:4")]
    Tall,
}
impl AspectPreset {
    /// Presets in the reference order.
    pub const ALL: [Self; 7] = [
        Self::Original,
        Self::Landscape,
        Self::Standard,
        Self::Wide,
        Self::Portrait,
        Self::Square,
        Self::Tall,
    ];
    /// Long edge 1920; `None` for `Original` until a first visual clip exists.
    pub const fn size(self) -> Option<CanvasSize> {
        Some(match self {
            Self::Original => return None,
            Self::Landscape => CanvasSize {
                width: 1920,
                height: 1080,
            },
            Self::Standard => CanvasSize {
                width: 1440,
                height: 1080,
            },
            Self::Wide => CanvasSize {
                width: 1920,
                height: 960,
            },
            Self::Portrait => CanvasSize {
                width: 1080,
                height: 1920,
            },
            Self::Square => CanvasSize {
                width: 1080,
                height: 1080,
            },
            Self::Tall => CanvasSize {
                width: 1080,
                height: 1440,
            },
        })
    }
    /// The preset's pixel size, resolving `Original` from the retained source size.
    pub const fn resolved(self, original: Option<CanvasSize>) -> Option<CanvasSize> {
        match self {
            Self::Original => original,
            _ => self.size(),
        }
    }
    /// Stable automation and wire fragment.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Landscape => "16x9",
            Self::Standard => "4x3",
            Self::Wide => "2x1",
            Self::Portrait => "9x16",
            Self::Square => "1x1",
            Self::Tall => "3x4",
        }
    }
    /// The reference's ratio text; `Original` is localized by the caller.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Landscape => "16:9",
            Self::Standard => "4:3",
            Self::Wide => "2:1",
            Self::Portrait => "9:16",
            Self::Square => "1:1",
            Self::Tall => "3:4",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub background: Color,
    /// Selected fixed preset; `None` after custom width/height entry.
    pub preset: Option<AspectPreset>,
    /// Last committed custom size, restored when the creator returns to custom.
    pub last_custom_size: CanvasSize,
}
const fn default_custom_size() -> CanvasSize {
    CanvasSize {
        width: 1920,
        height: 1080,
    }
}
impl Default for Canvas {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: FrameRate::FPS_30,
            background: Color([0, 0, 0, 255]),
            preset: Some(AspectPreset::Landscape),
            last_custom_size: default_custom_size(),
        }
    }
}
/// Output sample rate; presentation preferences stay in the app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub audio_sample_rate: u32,
    pub master_volume: f32,
    pub master_muted: bool,
    pub loudness: Loudness,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            audio_sample_rate: 48000,
            master_volume: 0.0,
            master_muted: false,
            loudness: Loudness::default(),
        }
    }
}
/// EBU R128 normalization target: integrated loudness and true-peak ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoudnessTarget {
    pub target_lufs: f32,
    pub true_peak_db: f32,
}
impl LoudnessTarget {
    /// EBU R128 streaming default: -14 LUFS integrated, -1 dBTP.
    pub const DEFAULT: Self = Self {
        target_lufs: -14.0,
        true_peak_db: -1.0,
    };
    /// Finite, on the hundredth grid: LUFS -50..=-5, true peak -20..=0 dBTP.
    pub fn is_valid(self) -> bool {
        let grid = |v: f32| (v * 100.0 - (v * 100.0).round()).abs() < 0.001;
        self.target_lufs.is_finite()
            && self.true_peak_db.is_finite()
            && (-50.0..=-5.0).contains(&self.target_lufs)
            && (-20.0..=0.0).contains(&self.true_peak_db)
            && grid(self.target_lufs)
            && grid(self.true_peak_db)
    }
}
/// Which part of the signal a loudness target normalizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoudnessScope {
    Clip,
    Master,
}
/// Which loudness gains a project requests; `measured` holds the last two-pass result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Loudness {
    /// Normalize each audible clip to this target before the master chain.
    pub clip: Option<LoudnessTarget>,
    /// Normalize the whole mix (after clip gains) to this target.
    pub master: Option<LoudnessTarget>,
    pub measured: Option<LoudnessGains>,
}
/// Gains derived by one measurement pass. `key` binds them to the audio content
/// and targets they were measured for; the mixer applies them unconditionally so
/// preview and export agree, and export admission refuses stale keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoudnessGains {
    pub key: [u8; 32],
    /// Per-clip dB gain, sorted by item; absent items (silence, identity) are unity.
    pub clip: Vec<(ItemId, f32)>,
    /// Gain in dB applied to the summed mix before the limiter.
    pub master_db: f32,
}
impl LoudnessGains {
    pub const MAX_CLIPS: usize = 4096;
    pub const GAIN_RANGE_DB: std::ops::RangeInclusive<f32> = -60.0..=60.0;
    pub fn is_valid(&self) -> bool {
        self.clip.len() <= Self::MAX_CLIPS
            && Self::GAIN_RANGE_DB.contains(&self.master_db)
            && self.master_db.is_finite()
            && self
                .clip
                .iter()
                .all(|(_, g)| g.is_finite() && Self::GAIN_RANGE_DB.contains(g))
            && self.clip.windows(2).all(|w| match w {
                [a, b] => a.0.0 < b.0.0,
                _ => true,
            })
    }
    pub fn clip_db(&self, item: ItemId) -> f32 {
        self.clip
            .binary_search_by_key(&item.0, |(id, _)| id.0)
            .map_or(0.0, |i| self.clip.get(i).map_or(0.0, |(_, g)| *g))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bookmark {
    pub id: BookmarkId,
    pub start: Tick,
    pub duration: Tick,
    pub note: NfcText,
    pub color: Color,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
    Text,
    Sticker,
    Effect,
    Filter,
    Adjustment,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub id: TrackId,
    pub name: NfcText,
    pub kind: TrackKind,
    pub muted: bool,
    pub hidden: bool,
    pub locked: bool,
    pub items: Vec<Item>,
}
/// Tracks are grouped in screen order, with one mandatory main video track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    pub transitions: Vec<crate::Transition>,
    /// Topmost overlay first. Audio tracks belong in `audio` instead.
    pub overlays: Vec<Track>,
    pub main: Track,
    /// Main-track magnet preference; structural gap closing is an engine edit policy.
    pub magnet: bool,
    pub audio: Vec<Track>,
}
impl Default for Timeline {
    fn default() -> Self {
        Self {
            transitions: Vec::new(),
            overlays: Vec::new(),
            main: Track {
                id: TrackId(0),
                name: "Main Track".into(),
                kind: TrackKind::Video,
                muted: false,
                hidden: false,
                locked: false,
                items: Vec::new(),
            },
            audio: Vec::new(),
            magnet: true,
        }
    }
}
impl Timeline {
    /// Bottom-to-top compositing order; reverse for screen order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Track> {
        self.audio
            .iter()
            .rev()
            .chain(std::iter::once(&self.main))
            .chain(self.overlays.iter().rev())
    }
    /// Mutable traversal in the same order as `iter`.
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut Track> {
        self.audio
            .iter_mut()
            .rev()
            .chain(std::iter::once(&mut self.main))
            .chain(self.overlays.iter_mut().rev())
    }
    pub fn track_count(&self) -> usize {
        self.overlays.len() + 1 + self.audio.len()
    }
    /// Insert a secondary track at the highest position for its group.
    pub fn insert_track(&mut self, track: Track) {
        if track.kind == TrackKind::Audio {
            self.audio.insert(0, track);
        } else {
            self.overlays.insert(0, track);
        }
    }
}
impl TrackKind {
    pub fn can_mute(self) -> bool {
        matches!(self, Self::Video | Self::Audio)
    }
    pub fn can_hide(self) -> bool {
        self != Self::Audio
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VideoParams {
    pub source_audio_enabled: bool,
    pub volume: f32,
    pub muted: bool,
    #[serde(default)]
    pub fades: crate::Fades,
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
impl LibrarySound {
    fn validate(&self) -> Result<()> {
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
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioParams {
    pub volume: f32,
    pub muted: bool,
    #[serde(default)]
    pub source: AudioSource,
    #[serde(default)]
    pub fades: crate::Fades,
}
/// Percentages of the canvas; `(50, 84.5)` reproduces the original lower-third box.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextPosition {
    pub x: f32,
    pub y: f32,
}
impl Default for TextPosition {
    fn default() -> Self {
        Self { x: 50.0, y: 84.5 }
    }
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
    pub background: Option<TextBackground>,
    pub position: TextPosition,
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
            weight: crate::typography::DEFAULT_WEIGHT,
            italic: false,
            align: TextAlign::default(),
            decoration: TextDecoration::default(),
            letter_spacing: 0.0,
            line_height: 0.0,
            color: Color([255; 4]),
            background: None,
            position: TextPosition::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionParams {
    pub cues: CueTrack,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OverlayParams {
    pub color: Color,
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
}
impl ItemParams {
    pub fn kind(&self) -> TrackKind {
        match self {
            Self::Video(_) => TrackKind::Video,
            Self::Audio(_) => TrackKind::Audio,
            Self::Text(_) => TrackKind::Text,
            Self::Caption(_) => TrackKind::Text,
            Self::Overlay(_) => TrackKind::Video,
        }
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
}

/// One independently editable timeline; canvas and assets remain project-wide.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub id: SceneId,
    pub name: NfcText,
    pub is_main: bool,
    pub timeline: Timeline,
    pub bookmarks: Vec<Bookmark>,
}
impl Scene {
    /// End of content in output ticks; bookmarks do not extend the scene.
    pub fn duration(&self) -> Tick {
        self.timeline
            .iter()
            .flat_map(|t| &t.items)
            .filter_map(|i| i.end().ok())
            .max()
            .unwrap_or(Tick::ZERO)
    }
}
/// Explicit export selection; output UI and adapters choose output paths separately.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "scope",
    content = "scenes",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SceneScope {
    Active,
    All,
    Selected(Vec<SceneId>),
}
/// Nonempty scene array with exactly one main scene. Main is never deletable.
#[derive(Clone, Debug, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "Vec<Scene>")]
#[schemars(with = "Vec<Scene>")]
pub struct Scenes {
    main: Scene,
    others: Vec<Scene>,
}
impl TryFrom<Vec<Scene>> for Scenes {
    type Error = Error;
    fn try_from(mut scenes: Vec<Scene>) -> Result<Self> {
        if scenes.len() > 256 {
            return Err(Error::TooLarge("scenes"));
        }
        if scenes.iter().filter(|s| s.is_main).count() != 1 {
            return Err(Error::Invalid("exactly one main scene required"));
        }
        let index = scenes
            .iter()
            .position(|s| s.is_main)
            .ok_or(Error::Invalid("main scene"))?;
        let main = scenes.remove(index);
        Ok(Self {
            main,
            others: scenes,
        })
    }
}
impl Serialize for Scenes {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}
impl Default for Scenes {
    fn default() -> Self {
        Self {
            main: Scene {
                id: SceneId(0),
                name: "Main scene".into(),
                is_main: true,
                timeline: Timeline::default(),
                bookmarks: Vec::new(),
            },
            others: Vec::new(),
        }
    }
}
impl Scenes {
    pub fn iter(&self) -> impl Iterator<Item = &Scene> {
        std::iter::once(&self.main).chain(&self.others)
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Scene> {
        std::iter::once(&mut self.main).chain(&mut self.others)
    }
    pub fn main(&self) -> &Scene {
        &self.main
    }
    pub fn get(&self, id: SceneId) -> Option<&Scene> {
        self.iter().find(|s| s.id == id)
    }
    pub fn get_mut(&mut self, id: SceneId) -> Option<&mut Scene> {
        self.iter_mut().find(|s| s.id == id)
    }
    pub fn insert(&mut self, scene: Scene) -> Result<()> {
        if scene.is_main || self.get(scene.id).is_some() {
            return Err(Error::Invalid("scene identity"));
        }
        if self.others.len() >= 255 {
            return Err(Error::TooLarge("scenes"));
        }
        self.others.push(scene);
        Ok(())
    }
    pub fn delete(&mut self, ids: &BTreeSet<SceneId>) -> Result<()> {
        if ids.contains(&self.main.id) {
            return Err(Error::Invalid("main scene cannot be deleted"));
        }
        if ids.iter().any(|id| self.get(*id).is_none()) {
            return Err(Error::Invalid("scene not found"));
        }
        self.others.retain(|s| !ids.contains(&s.id));
        Ok(())
    }
}
/// Editable candidate data; committing through `Project::new` validates the whole snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectData {
    pub id: ProjectId,
    pub name: NfcText,
    pub canvas: Canvas,
    pub original_canvas_size: Option<CanvasSize>,
    pub assets: Vec<SourceAsset>,
    pub scenes: Scenes,
    pub current_scene_id: SceneId,
    pub settings: Settings,
}
impl ProjectData {
    /// Round every stored canvas size to even through `CanvasSize::rounded_even`
    /// (files written before custom sizes rounded may hold odd ones). A size that
    /// cannot round inside the bounds stays as stored; validation judges it.
    pub fn round_canvas_even(&mut self) {
        let even = |size: CanvasSize| size.rounded_even().filter(|s| s.is_valid()).unwrap_or(size);
        let canvas = even(CanvasSize {
            width: self.canvas.width,
            height: self.canvas.height,
        });
        self.canvas.width = canvas.width;
        self.canvas.height = canvas.height;
        self.canvas.last_custom_size = even(self.canvas.last_custom_size);
        self.original_canvas_size = self.original_canvas_size.map(even);
    }
    /// Candidates with an invalid current ID read main; validation still refuses them.
    pub fn scene(&self) -> &Scene {
        self.scenes
            .get(self.current_scene_id)
            .unwrap_or(self.scenes.main())
    }
    pub fn scene_mut(&mut self) -> &mut Scene {
        if self.current_scene_id == self.scenes.main.id {
            return &mut self.scenes.main;
        }
        self.scenes
            .others
            .iter_mut()
            .find(|s| s.id == self.current_scene_id)
            .unwrap_or(&mut self.scenes.main)
    }
}
/// An immutable validated snapshot. Commands build a candidate and commit once.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "ProjectData")]
pub struct Project(ProjectData);
impl TryFrom<ProjectData> for Project {
    type Error = Error;
    fn try_from(data: ProjectData) -> Result<Self> {
        Self::new(data)
    }
}
impl Project {
    pub fn new(data: ProjectData) -> Result<Self> {
        validate(&data)?;
        Ok(Self(data))
    }
    pub fn empty(id: ProjectId, name: NfcText) -> Result<Self> {
        Self::new(ProjectData {
            id,
            name,
            canvas: Canvas::default(),
            original_canvas_size: None,
            assets: Vec::new(),
            scenes: Scenes::default(),
            current_scene_id: SceneId(0),
            settings: Settings::default(),
        })
    }
    pub fn data(&self) -> &ProjectData {
        &self.0
    }
    /// Copy this document under a fresh identity, retaining all editable content.
    /// Entropy failure leaves this snapshot untouched.
    pub fn fork(&self) -> Result<Self> {
        let mut data = self.0.clone();
        loop {
            let mut bytes = [0; 8];
            getrandom::fill(&mut bytes).map_err(Error::Identity)?;
            data.id = ProjectId(u64::from_le_bytes(bytes));
            if data.id != self.0.id {
                return Self::new(data);
            }
        }
    }
    pub fn into_data(self) -> ProjectData {
        self.0
    }
    pub fn duration(&self) -> Tick {
        self.0.scene().duration()
    }
    /// Validate the complete explicit export scope before scheduling any output.
    pub fn scene_ids(&self, scope: &SceneScope) -> Result<Vec<SceneId>> {
        match scope {
            SceneScope::Active => Ok(vec![self.0.current_scene_id]),
            SceneScope::All => Ok(self.0.scenes.iter().map(|s| s.id).collect()),
            SceneScope::Selected(ids) => {
                if ids.is_empty() || ids.len() > 256 {
                    return Err(Error::Invalid("scene export selection"));
                }
                let mut seen = BTreeSet::new();
                if ids
                    .iter()
                    .any(|id| self.0.scenes.get(*id).is_none() || !seen.insert(*id))
                {
                    return Err(Error::Invalid("scene export selection"));
                }
                Ok(ids.clone())
            }
        }
    }
    /// Immutable scene-specific snapshot for the shared preview/export pipeline.
    pub fn for_scene(&self, scene: SceneId) -> Result<Self> {
        if self.0.scenes.get(scene).is_none() {
            return Err(Error::Invalid("scene not found"));
        }
        let mut data = self.0.clone();
        data.current_scene_id = scene;
        Self::new(data)
    }
}
fn text(value: &NfcText) -> Result<()> {
    if value.as_str().len() > 1 << 20 {
        return Err(Error::TooLarge("text"));
    }
    Ok(())
}
fn validate(data: &ProjectData) -> Result<()> {
    text(&data.name)?;
    let c = &data.canvas;
    let canvas_size = CanvasSize {
        width: c.width,
        height: c.height,
    };
    if !canvas_size.is_valid() {
        return Err(Error::Invalid("canvas dimensions"));
    }
    if !c.last_custom_size.is_valid() {
        return Err(Error::Invalid("last custom canvas dimensions"));
    }
    if data
        .original_canvas_size
        .is_some_and(|size| !size.is_valid())
    {
        return Err(Error::Invalid("original canvas dimensions"));
    }
    if c.preset == Some(AspectPreset::Original) && data.original_canvas_size.is_none() {
        return Err(Error::Invalid("canvas original preset"));
    }
    // Exact frame ticks are required by edit and render consumers.
    let frame = c.fps.frame_duration()?;
    if frame <= Tick::ZERO
        || (i128::from(time::TICKS_PER_SECOND) * i128::from(c.fps.den())) % i128::from(c.fps.num())
            != 0
    {
        return Err(Error::Unsupported(
            "canvas rate must have exact positive frame ticks",
        ));
    }
    if !valid_volume(data.settings.master_volume) {
        return Err(Error::Invalid("master volume"));
    }
    if !(8000..=192000).contains(&data.settings.audio_sample_rate) {
        return Err(Error::Invalid("audio sample rate"));
    }
    let loudness = &data.settings.loudness;
    if loudness.clip.is_some_and(|t| !t.is_valid())
        || loudness.master.is_some_and(|t| !t.is_valid())
        || loudness.measured.as_ref().is_some_and(|m| !m.is_valid())
    {
        return Err(Error::Invalid("loudness settings"));
    }
    if data.assets.len() > 10000
        || data
            .scenes
            .iter()
            .map(|s| s.timeline.track_count())
            .sum::<usize>()
            > 256
        || data.scenes.iter().map(|s| s.bookmarks.len()).sum::<usize>() > 100000
    {
        return Err(Error::TooLarge("project collections"));
    }
    let mut assets = BTreeMap::new();
    for asset in &data.assets {
        if assets.insert(asset.id(), asset).is_some() {
            return Err(Error::Invalid("duplicate asset ID"));
        }
    }
    if data.scenes.get(data.current_scene_id).is_none() {
        return Err(Error::Invalid("current scene not found"));
    }
    if !data.scenes.main().is_main || data.scenes.iter().filter(|s| s.is_main).count() != 1 {
        return Err(Error::Invalid("main scene invariant"));
    }
    let mut scene_ids = BTreeSet::new();
    for scene in data.scenes.iter() {
        text(&scene.name)?;
        if !scene_ids.insert(scene.id) {
            return Err(Error::Invalid("duplicate scene ID"));
        }
        if scene.timeline.main.kind != TrackKind::Video
            || scene
                .timeline
                .overlays
                .iter()
                .any(|t| t.kind == TrackKind::Audio)
            || scene
                .timeline
                .audio
                .iter()
                .any(|t| t.kind != TrackKind::Audio)
        {
            return Err(Error::Invalid("track layout"));
        }
    }
    let mut transition_ids = BTreeSet::new();
    for scene in data.scenes.iter() {
        let transitions = &scene.timeline.transitions;
        if transition_ids.len().saturating_add(transitions.len()) > 100_000 {
            return Err(Error::TooLarge("transitions"));
        }
        let mut cuts = BTreeSet::new();
        let mut spans = Vec::with_capacity(transitions.len());
        for tr in transitions {
            tr.validate(&scene.timeline, data)?;
            if !cuts.insert((tr.track, tr.from, tr.to)) {
                return Err(Error::Invalid("duplicate transition cut"));
            }
            if !transition_ids.insert(tr.id) {
                return Err(Error::Invalid("duplicate transition ID"));
            }
            spans.push((tr.track, tr.start, tr.end()?));
        }
        spans.sort();
        if spans.windows(2).any(|p| {
            p.first()
                .zip(p.get(1))
                .is_some_and(|(a, b)| a.0 == b.0 && a.2 > b.1)
        }) {
            return Err(Error::Invalid("overlapping transitions"));
        }
    }
    let mut track_ids = BTreeSet::new();
    let mut item_ids = BTreeSet::new();
    let mut keys = 0usize;
    for track in data.scenes.iter().flat_map(|s| s.timeline.iter()) {
        text(&track.name)?;
        if !track_ids.insert(track.id) {
            return Err(Error::Invalid("duplicate track ID"));
        }
        let mut end = Tick::ZERO;
        for item in &track.items {
            if item.blend_mode != crate::BlendMode::Normal
                && matches!(item.content, ItemParams::Audio(_))
            {
                return Err(Error::Invalid("visual blend on audio item"));
            }
            if !item.effects.is_empty() && matches!(item.content, ItemParams::Audio(_)) {
                return Err(Error::Invalid("visual effect on audio item"));
            }
            if item.effects.len() > 64 {
                return Err(Error::TooLarge("item effects"));
            }
            let mut effect_ids = BTreeSet::new();
            for effect in &item.effects {
                effect.validate()?;
                if !effect_ids.insert(&effect.id) {
                    return Err(Error::Invalid("duplicate effect id"));
                }
                if let Some(param) = item.animations.get(&effect.intensity_path()?)
                    && !matches!(param.value_at(Tick::ZERO)?, crate::ParamValue::Scalar(_))
                {
                    return Err(Error::Invalid("blur animation must be scalar"));
                }
            }
            if item.masks.len() > 1024 {
                return Err(Error::TooLarge("item masks"));
            }
            if !item.masks.is_empty() && matches!(item.content, ItemParams::Audio(_)) {
                return Err(Error::Invalid("mask on audio item"));
            }
            let mut mask_ids = BTreeSet::new();
            for mask in &item.masks {
                mask.validate()?;
                if !mask_ids.insert(&mask.id) {
                    return Err(Error::Invalid("duplicate mask id"));
                }
                for key in crate::MASK_SCALARS {
                    if let Some(param) = item.animations.get(&mask.path(key)?)
                        && !matches!(param.value_at(Tick::ZERO)?, crate::ParamValue::Scalar(_))
                    {
                        return Err(Error::Invalid("mask animation must be scalar"));
                    }
                }
            }
            item.animations.validate()?;
            keys = keys
                .checked_add(item.animations.key_count())
                .ok_or(Error::TooLarge("project keys"))?;
            if keys > crate::MAX_PROJECT_KEYS {
                return Err(Error::TooLarge("project keys"));
            }
            if !item_ids.insert(item.id) {
                return Err(Error::Invalid("duplicate item ID"));
            }
            if item_ids.len() > 100000 {
                return Err(Error::TooLarge("items"));
            }
            if item.start < end {
                return Err(Error::Invalid("unsorted or overlapping items"));
            }
            end = item.end()?;
            if item.source_range.start() < Tick::ZERO || item.content.kind() != track.kind {
                return Err(Error::Invalid("source range or track kind"));
            }
            if matches!(item.content, ItemParams::Video(_) | ItemParams::Audio(_)) {
                item.validate_retime_window()?;
                if item.duration()? <= Tick::ZERO {
                    return Err(Error::Invalid("retimed item shorter than one tick"));
                }
            } else if item.retime != crate::Retime::default() {
                return Err(Error::Invalid("retime on non-media item"));
            }
            let asset = item
                .source_asset
                .map(|id| {
                    assets
                        .get(&id)
                        .copied()
                        .ok_or(Error::Invalid("missing asset reference"))
                })
                .transpose()?;
            match &item.content {
                ItemParams::Video(_) | ItemParams::Audio(_) => {
                    let asset = asset.ok_or(Error::Invalid("media item requires source"))?;
                    let kind = if track.kind == TrackKind::Video {
                        StreamKind::Video
                    } else {
                        StreamKind::Audio
                    };
                    if !asset.probe().streams.iter().any(|s| s.kind == kind) {
                        return Err(Error::Invalid("asset lacks item stream kind"));
                    }
                    if asset.probe().duration.is_some_and(|d| {
                        item.source_range.end() > d
                            || item.retime.window.is_some_and(|w| w.source_range.end() > d)
                    }) {
                        return Err(Error::Invalid("item exceeds source duration"));
                    }
                }
                ItemParams::Text(p) => {
                    text(&p.content)?;
                    text(&p.font)?;
                    if !crate::typography::valid(
                        p.font.as_str(),
                        p.weight,
                        p.letter_spacing,
                        p.line_height,
                    ) || !p.font_size.is_finite()
                        || !(1.0..=4096.0).contains(&p.font_size)
                        || !p.position.x.is_finite()
                        || !p.position.y.is_finite()
                        || !(0.0..=100.0).contains(&p.position.x)
                        || !(0.0..=100.0).contains(&p.position.y)
                        || p.background.as_ref().is_some_and(|b| !b.valid())
                    {
                        return Err(Error::Invalid("text style"));
                    }
                }
                ItemParams::Caption(p) => {
                    if asset.is_none() {
                        return Err(Error::Invalid("caption item requires source identity"));
                    }
                    if p.cues.cues.len() > 100000 {
                        return Err(Error::TooLarge("cues"));
                    }
                    let mut ids = BTreeSet::new();
                    for cue in &p.cues.cues {
                        text(&cue.id)?;
                        if !ids.insert(&cue.id) {
                            return Err(Error::Invalid("duplicate cue ID"));
                        }
                        if cue.range.start() < item.source_range.start()
                            || cue.range.end() > item.source_range.end()
                        {
                            return Err(Error::Invalid("cue exceeds source range"));
                        }
                        for t in cue.texts.values() {
                            text(&t.text)?;
                            if let Some(reference) = &t.translation_ref {
                                text(reference)?;
                            }
                        }
                        if let Some(style) = &cue.style
                            && !style.is_valid()
                        {
                            return Err(Error::Invalid("cue style"));
                        }
                    }
                }
                ItemParams::Overlay(_) => {}
            }
            if matches!(item.content, ItemParams::Text(_) | ItemParams::Overlay(_))
                && asset.is_some()
            {
                return Err(Error::Invalid("generated item cannot reference media"));
            }
            if let ItemParams::Video(p) = &item.content
                && !valid_volume(p.volume)
            {
                return Err(Error::Invalid("video volume"));
            }
            if let ItemParams::Video(p) = &item.content {
                p.fades.validate()?;
            }
            if let ItemParams::Audio(p) = &item.content {
                if !valid_volume(p.volume) {
                    return Err(Error::Invalid("audio gain"));
                }
                p.fades.validate()?;
                if let AudioSource::Library { sound } = &p.source {
                    sound.validate()?;
                }
            }
        }
    }
    let mut bookmark_ids = BTreeSet::new();
    for b in data.scenes.iter().flat_map(|s| &s.bookmarks) {
        text(&b.note)?;
        if !bookmark_ids.insert(b.id)
            || b.start < Tick::ZERO
            || b.duration < Tick::ZERO
            || b.start.checked_add(b.duration).is_none()
        {
            return Err(Error::Invalid("bookmark"));
        }
    }
    Ok(())
}

/// Clip/master dB range and hundredth-dB editing grid. Curves interpolate continuously.
pub fn valid_volume(value: f32) -> bool {
    value.is_finite()
        && (-60.0..=20.0).contains(&value)
        && (value * 100.0 - (value * 100.0).round()).abs() < 0.001
}
