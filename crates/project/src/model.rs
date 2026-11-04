use crate::{Error, Result};
use captions::{CueTrack, NfcText};
use media::{AssetId, SourceAsset, StreamKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::{FrameRate, SourceRange, Tick};

macro_rules! id {
    ($($name:ident),+) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);
    )+};
}
id!(ProjectId, TrackId, ItemId, BookmarkId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Color(pub [u8; 4]);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: FrameRate,
    pub background: Color,
}
impl Default for Canvas {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: FrameRate::FPS_30,
            background: Color([0, 0, 0, 255]),
        }
    }
}
/// Output sample rate; presentation preferences stay in the app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub audio_sample_rate: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            audio_sample_rate: 48000,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bookmark {
    pub id: BookmarkId,
    pub start: Tick,
    pub duration: Tick,
    pub note: NfcText,
    pub color: Color,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
    Text,
    Caption,
    Overlay,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoParams {
    pub source_audio_enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioParams {
    pub gain_db: f32,
    pub muted: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextParams {
    pub content: NfcText,
    pub font_size: f32,
    pub color: Color,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptionParams {
    pub cues: CueTrack,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayParams {
    pub color: Color,
}
/// Typed parameters determine item kind; no unvalidated arbitrary parameter bag.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
            Self::Caption(_) => TrackKind::Caption,
            Self::Overlay(_) => TrackKind::Overlay,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: ItemId,
    pub source_asset: Option<AssetId>,
    /// Source ticks for media/captions; local content ticks for generated items.
    pub source_range: SourceRange,
    /// Output timeline ticks. No retiming in this schema: output duration equals source duration.
    pub start: Tick,
    pub content: ItemParams,
}
impl Item {
    pub fn end(&self) -> Result<Tick> {
        self.start
            .checked_add(self.source_range.duration())
            .ok_or(Error::Invalid("item end overflow"))
    }
}

/// Editable candidate data; committing through `Project::new` validates the whole snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectData {
    pub id: ProjectId,
    pub name: NfcText,
    pub canvas: Canvas,
    pub assets: Vec<SourceAsset>,
    pub timeline: Timeline,
    pub bookmarks: Vec<Bookmark>,
    pub settings: Settings,
}
/// An immutable validated snapshot. Commands build a candidate and commit once.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
            assets: Vec::new(),
            timeline: Timeline::default(),
            bookmarks: Vec::new(),
            settings: Settings::default(),
        })
    }
    pub fn data(&self) -> &ProjectData {
        &self.0
    }
    pub fn into_data(self) -> ProjectData {
        self.0
    }
    pub fn duration(&self) -> Tick {
        self.0
            .timeline
            .tracks
            .iter()
            .flat_map(|t| &t.items)
            .filter_map(|i| i.end().ok())
            .max()
            .unwrap_or(Tick::ZERO)
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
    if c.width == 0 || c.height == 0 || u64::from(c.width) * u64::from(c.height) > 1 << 26 {
        return Err(Error::Invalid("canvas dimensions"));
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
    if !(8000..=192000).contains(&data.settings.audio_sample_rate) {
        return Err(Error::Invalid("audio sample rate"));
    }
    if data.assets.len() > 10000
        || data.timeline.tracks.len() > 256
        || data.bookmarks.len() > 100000
    {
        return Err(Error::TooLarge("project collections"));
    }
    let mut assets = BTreeMap::new();
    for asset in &data.assets {
        if assets.insert(asset.id(), asset).is_some() {
            return Err(Error::Invalid("duplicate asset ID"));
        }
    }
    let mut track_ids = BTreeSet::new();
    let mut item_ids = BTreeSet::new();
    for track in &data.timeline.tracks {
        text(&track.name)?;
        if !track_ids.insert(track.id) {
            return Err(Error::Invalid("duplicate track ID"));
        }
        let mut end = Tick::ZERO;
        for item in &track.items {
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
                    if asset
                        .probe()
                        .duration
                        .is_some_and(|d| item.source_range.end() > d)
                    {
                        return Err(Error::Invalid("item exceeds source duration"));
                    }
                }
                ItemParams::Text(p) => {
                    text(&p.content)?;
                    if !p.font_size.is_finite() || !(1.0..=4096.0).contains(&p.font_size) {
                        return Err(Error::Invalid("text font size"));
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
                    }
                }
                ItemParams::Overlay(_) => {}
            }
            if matches!(item.content, ItemParams::Text(_) | ItemParams::Overlay(_))
                && asset.is_some()
            {
                return Err(Error::Invalid("generated item cannot reference media"));
            }
            if let ItemParams::Audio(p) = &item.content
                && (!p.gain_db.is_finite() || !(-120.0..=24.0).contains(&p.gain_db))
            {
                return Err(Error::Invalid("audio gain"));
            }
        }
    }
    let mut bookmark_ids = BTreeSet::new();
    for b in &data.bookmarks {
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
