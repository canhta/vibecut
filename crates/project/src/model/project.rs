//! The validated project snapshot, its editable candidate and whole-project invariants.
use super::item::{AudioSource, ItemParams, TextStyle, valid_volume};
use super::scene::{Scene, SceneId, SceneScope, Scenes};
use super::settings::{AspectPreset, Canvas, CanvasSize, Settings};
use super::track::TrackKind;
use crate::{Error, Result};
use captions::NfcText;
use media::{SourceAsset, StreamKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::Tick;

id!(CaptionPresetId);
id!(TextPresetId);

/// Maximum number of named caption values in one project.
pub const MAX_CAPTION_PRESETS: usize = 64;
/// Maximum number of named text styles in one project.
pub const MAX_TEXT_PRESETS: usize = 64;

/// Whether a preset name is storable: 1..=64 extended graphemes, at most 256
/// UTF-8 bytes, no controls, no leading or trailing whitespace. Input is never trimmed.
fn valid_preset_name(name: &NfcText) -> bool {
    let name = name.as_str();
    !name.is_empty()
        && name.len() <= 256
        && name.trim() == name
        && !name.chars().any(char::is_control)
        && captions::NormalizedText::new(name)
            .is_ok_and(|text| (1..=64).contains(&text.spans().len()))
}

/// A reusable value; application copies the style into ordinary cue styles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavedCaptionPreset {
    pub id: CaptionPresetId,
    pub name: NfcText,
    pub style: captions::CueStyle,
}
impl SavedCaptionPreset {
    /// Names contain 1..=64 extended graphemes, at most 256 UTF-8 bytes,
    /// no controls, and no leading or trailing whitespace. Input is never trimmed.
    pub fn valid_name(name: &NfcText) -> bool {
        valid_preset_name(name)
    }
}

/// A reusable text look; application copies values into ordinary text parameters.
/// Content and animation keys never belong to a preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavedTextPreset {
    pub id: TextPresetId,
    pub name: NfcText,
    pub style: TextStyle,
}
impl SavedTextPreset {
    /// The same closed naming rule as caption presets.
    pub fn valid_name(name: &NfcText) -> bool {
        valid_preset_name(name)
    }
}

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

/// Editable candidate data; committing through `Project::new` validates the whole snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectData {
    pub id: ProjectId,
    pub name: NfcText,
    pub canvas: Canvas,
    pub original_canvas_size: Option<CanvasSize>,
    pub assets: Vec<SourceAsset>,
    pub luts: Vec<crate::LutRecord>,
    pub scenes: Scenes,
    pub current_scene_id: SceneId,
    pub settings: Settings,
    pub caption_presets: Vec<SavedCaptionPreset>,
    pub text_presets: Vec<SavedTextPreset>,
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
            luts: Vec::new(),
            scenes: Scenes::default(),
            current_scene_id: SceneId(0),
            settings: Settings::default(),
            caption_presets: Vec::new(),
            text_presets: Vec::new(),
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
pub(super) fn text(value: &NfcText) -> Result<()> {
    if value.as_str().len() > 1 << 20 {
        return Err(Error::TooLarge("text"));
    }
    Ok(())
}
fn validate(data: &ProjectData) -> Result<()> {
    text(&data.name)?;
    if data.caption_presets.len() > MAX_CAPTION_PRESETS {
        return Err(Error::TooLarge("caption presets"));
    }
    let mut preset_ids = BTreeSet::new();
    let mut preset_names = BTreeSet::new();
    for preset in &data.caption_presets {
        if !SavedCaptionPreset::valid_name(&preset.name)
            || !preset.style.is_valid()
            || !preset_ids.insert(preset.id)
            || !preset_names.insert(&preset.name)
        {
            return Err(Error::Invalid("caption preset"));
        }
    }
    if data.text_presets.len() > MAX_TEXT_PRESETS {
        return Err(Error::TooLarge("text presets"));
    }
    let mut text_preset_ids = BTreeSet::new();
    let mut text_preset_names = BTreeSet::new();
    for preset in &data.text_presets {
        if !SavedTextPreset::valid_name(&preset.name)
            || !preset.style.valid()
            || !text_preset_ids.insert(preset.id)
            || !text_preset_names.insert(&preset.name)
        {
            return Err(Error::Invalid("text preset"));
        }
    }
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
    if frame <= Tick::ZERO || !c.fps.has_exact_frame_ticks() {
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
    if data.luts.len() > 16 {
        return Err(Error::TooLarge("project LUTs"));
    }
    let mut luts = BTreeSet::new();
    for record in &data.luts {
        record.validate()?;
        if !luts.insert(record.id) {
            return Err(Error::Invalid("duplicate LUT"));
        }
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
                for value in effect.params.values() {
                    if let crate::EffectValue::Lut(Some(id)) = value {
                        if !luts.contains(id) {
                            return Err(Error::Invalid("unregistered LUT"));
                        }
                    }
                }
                if !effect_ids.insert(&effect.id) {
                    return Err(Error::Invalid("duplicate effect id"));
                }
                effect.validate_animations(&item.animations)?;
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
            crate::Crop::validate_animations(
                match &item.content {
                    ItemParams::Video(p) => p.crop.as_ref(),
                    _ => None,
                },
                &item.animations,
            )?;
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
                    if !crate::visual::valid(
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
                    if p.cues.styles.len() > captions::MAX_CUE_STYLES {
                        return Err(Error::TooLarge("cue styles"));
                    }
                    for style in &p.cues.styles {
                        if !style.is_valid() {
                            return Err(Error::Invalid("cue style"));
                        }
                    }
                    let mut ids = BTreeSet::new();
                    for cue in &p.cues.cues {
                        cue.word_timing
                            .validate()
                            .map_err(|_| Error::Invalid("cue word correspondence"))?;
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
                        if cue
                            .style
                            .is_some_and(|id| usize::from(id.0) >= p.cues.styles.len())
                        {
                            return Err(Error::Invalid("cue style reference"));
                        }
                    }
                }
                ItemParams::Overlay(_) => {}
                ItemParams::Graphic(p) => p.validate(&item.animations)?,
            }
            if matches!(
                item.content,
                ItemParams::Text(_) | ItemParams::Overlay(_) | ItemParams::Graphic(_)
            ) && asset.is_some()
            {
                return Err(Error::Invalid("generated item cannot reference media"));
            }
            if let ItemParams::Video(p) = &item.content
                && (!valid_volume(p.volume) || p.crop.as_ref().is_some_and(|crop| !crop.valid()))
            {
                return Err(Error::Invalid("video parameters"));
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
