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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanvasSize {
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub audio_sample_rate: u32,
    pub master_volume: f32,
    pub master_muted: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            audio_sample_rate: 48000,
            master_volume: 0.0,
            master_muted: false,
        }
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
    /// Project canvas pixels; scaled with the output like the font size.
    pub padding: f32,
    pub radius: f32,
}
impl Default for TextBackground {
    fn default() -> Self {
        Self {
            color: Color([0, 0, 0, 160]),
            padding: 12.0,
            radius: 4.0,
        }
    }
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
    if c.width == 0 || c.height == 0 || u64::from(c.width) * u64::from(c.height) > 1 << 26 {
        return Err(Error::Invalid("canvas dimensions"));
    }
    if data.original_canvas_size.as_ref().is_some_and(|size| {
        size.width == 0
            || size.height == 0
            || u64::from(size.width) * u64::from(size.height) > 1 << 26
    }) {
        return Err(Error::Invalid("original canvas dimensions"));
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
        let mut spans = Vec::with_capacity(transitions.len());
        for tr in transitions {
            tr.validate(&scene.timeline, data)?;
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
                        || p.background.as_ref().is_some_and(|background| {
                            !background.padding.is_finite()
                                || !background.radius.is_finite()
                                || !(0.0..=128.0).contains(&background.padding)
                                || !(0.0..=128.0).contains(&background.radius)
                        })
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
