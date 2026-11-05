use captions::NfcText;
use media::{AssetId, SourceAsset};
use project::{BookmarkId, Color, ItemId, ItemParams, TrackId, TrackKind};
use serde::{Deserialize, Serialize};
use time::{SourceRange, Tick};

/// Secondary-track position in screen order. Group boundaries cannot be crossed.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum TrackPlacement {
    #[default]
    Auto,
    Above {
        track: TrackId,
    },
    Below {
        track: TrackId,
    },
}

/// Auto uses main video with magnet, otherwise the first free compatible track.
/// Explicit tracks refuse overlaps; above/below allocates a secondary track atomically.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Placement {
    #[default]
    Auto,
    Track {
        track: TrackId,
    },
    Above {
        track: TrackId,
    },
    Below {
        track: TrackId,
    },
}

/// Final location of an item allocated by a command, including nested batches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ItemPlacement {
    pub track: TrackId,
    pub item: ItemId,
}

/// A shared field edit; incompatible item kinds are left unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "field",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ItemParamPatch {
    #[serde(rename = "blendMode")]
    BlendMode(project::BlendMode),
    Opacity(f64),
    #[serde(rename = "volume")]
    AudioGain(f32),
    #[serde(rename = "muted")]
    AudioMuted(bool),
    TextContent(NfcText),
    TextFont(NfcText),
    TextWeight(u16),
    TextItalic(bool),
    TextLetterSpacing(f32),
    TextLineHeight(f32),
    TextSize(f32),
    TextColor(Color),
    TextBackground(Option<project::TextBackground>),
    TextBackgroundColor(Color),
    TextPositionX(f32),
    TextPositionY(f32),
    #[serde(rename = "transform.positionX")]
    PositionX(f64),
    #[serde(rename = "transform.positionY")]
    PositionY(f64),
    #[serde(rename = "transform.scaleX")]
    ScaleX(f64),
    #[serde(rename = "transform.scaleY")]
    ScaleY(f64),
    #[serde(rename = "transform.rotate")]
    Rotate(f64),
    GraphicFill(Color),
    GraphicStroke(Color),
}
impl ItemParamPatch {
    pub(crate) fn transform(&self) -> Option<(project::TransformField, f64)> {
        use project::TransformField as F;
        Some(match self {
            Self::PositionX(v) => (F::PositionX, *v),
            Self::PositionY(v) => (F::PositionY, *v),
            Self::ScaleX(v) => (F::ScaleX, *v),
            Self::ScaleY(v) => (F::ScaleY, *v),
            Self::Rotate(v) => (F::Rotate, *v),
            _ => return None,
        })
    }
    pub fn compatible(&self, content: &ItemParams) -> bool {
        if self.transform().is_some() || matches!(self, Self::BlendMode(_) | Self::Opacity(_)) {
            return !matches!(content, ItemParams::Audio(_));
        }
        matches!(
            (self, content),
            (
                Self::AudioGain(_) | Self::AudioMuted(_),
                ItemParams::Audio(_) | ItemParams::Video(_)
            ) | (
                Self::TextContent(_)
                    | Self::TextFont(_)
                    | Self::TextWeight(_)
                    | Self::TextItalic(_)
                    | Self::TextLetterSpacing(_)
                    | Self::TextLineHeight(_)
                    | Self::TextSize(_)
                    | Self::TextColor(_)
                    | Self::TextBackground(_)
                    | Self::TextBackgroundColor(_)
                    | Self::TextPositionX(_)
                    | Self::TextPositionY(_),
                ItemParams::Text(_)
            ) | (
                Self::GraphicFill(_) | Self::GraphicStroke(_),
                ItemParams::Overlay(_)
            )
        )
    }
    pub(crate) fn validate(&self) -> crate::Result<()> {
        if let Some((field, value)) = self.transform() {
            return if field.clamp(value).is_ok_and(|bounded| bounded == value) {
                Ok(())
            } else {
                Err(crate::EditError::Invalid("transform bounds"))
            };
        }
        let valid = match self {
            Self::BlendMode(_) => true,
            Self::Opacity(v) => v.is_finite() && (0.0..=1.0).contains(v),
            Self::AudioGain(v) => project::valid_volume(*v),
            Self::AudioMuted(_) => true,
            Self::TextContent(v) => v.as_str().len() <= 1 << 20,
            Self::TextFont(v) => !v.as_str().trim().is_empty() && v.as_str().len() <= 256,
            Self::TextWeight(v) => project::valid_weight(*v),
            Self::TextItalic(_) => true,
            Self::TextLetterSpacing(v) => project::valid_letter_spacing(*v),
            Self::TextLineHeight(v) => project::valid_line_height(*v),
            Self::TextSize(v) => v.is_finite() && (1.0..=4096.0).contains(v),
            Self::TextColor(_)
            | Self::TextBackgroundColor(_)
            | Self::GraphicFill(_)
            | Self::GraphicStroke(_) => true,
            Self::TextBackground(background) => background.as_ref().is_none_or(|background| {
                background.padding.is_finite()
                    && background.radius.is_finite()
                    && (0.0..=128.0).contains(&background.padding)
                    && (0.0..=128.0).contains(&background.radius)
            }),
            Self::TextPositionX(v) | Self::TextPositionY(v) => {
                v.is_finite() && (0.0..=100.0).contains(v)
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(crate::EditError::Invalid("shared field value"))
        }
    }
}

/// Effects are engine metadata; adapters must never infer permission from names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Edit,
    SessionControl,
    FilePublish,
    Read,
    JobControl,
}

/// Generated from the typed registry, including closed nested model schemas.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub effect: Effect,
    pub limits: crate::control::Limits,
    pub params_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub example: Example,
}
/// Fixture requirements are registry data; conformance consumers never switch on IDs.
#[derive(Clone, Copy, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fixture {
    TransitionCut,
    Transition,
    Timeline,
    /// Timeline plus one audible clip, item 5.
    Audio,
    Library,
    History,
    Redo,
    Gesture,
    Job,
    Create,
    Save,
}
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
pub struct Example {
    pub fixture: Fixture,
    pub params: serde_json::Value,
}
impl CommandSpec {
    /// Coarse session availability. Target/parameter validation occurs at execution.
    pub fn enabled(&self, session: &crate::Session) -> bool {
        match self.id {
            "export_subtitles" | "burn_subtitles" => {
                !session.transaction_active()
                    && session.project().data().scene().timeline.iter().any(|t| {
                        t.items
                            .iter()
                            .any(|i| matches!(i.content, ItemParams::Caption(_)))
                    })
            }
            "begin_gesture" => !session.transaction_active(),
            "commit_gesture" | "cancel_gesture" => session.gesture_active(),
            "preview.snapshot" => {
                !session.transaction_active() && session.project().duration() > Tick::ZERO
            }
            "undo" => session.can_undo(),
            "redo" => session.can_redo(),
            "track.set_flags" | "track.set_muted" | "track.set_hidden" | "track.rename"
            | "track.reorder" | "track.set_locked" => true,
            "media.remove" => !session.project().data().assets.is_empty(),
            "timeline.set_speed" => session.project().data().scene().timeline.iter().any(|t| {
                !t.locked
                    && t.items
                        .iter()
                        .any(|i| matches!(i.content, ItemParams::Video(_) | ItemParams::Audio(_)))
            }),
            "track.remove" => session
                .project()
                .data()
                .scene()
                .timeline
                .overlays
                .iter()
                .chain(&session.project().data().scene().timeline.audio)
                .any(|t| !t.locked),
            "effects.add"
            | "effects.remove"
            | "effects.update_param"
            | "effects.reorder"
            | "effects.toggle"
            | "masks.add"
            | "masks.remove"
            | "masks.update"
            | "masks.reorder" => session.project().data().scene().timeline.iter().any(|t| {
                !t.locked
                    && t.items
                        .iter()
                        .any(|i| !matches!(i.content, ItemParams::Audio(_)))
            }),
            "move_item"
            | "move_items"
            | "split_at"
            | "trim_start"
            | "trim_end"
            | "delete"
            | "ripple_delete"
            | "set_item_param"
            | "timeline.set_item_param"
            | "audio.set_fade"
            | "retranslate"
            | "keyframes.set_param"
            | "keyframes.add"
            | "keyframes.remove"
            | "keyframes.retime"
            | "keyframes.set_handle"
            | "keyframes.scale_time" => session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .any(|t| !t.locked && !t.items.is_empty()),
            _ => true,
        }
    }
}
macro_rules! commands {
    ($($variant:ident, $id:literal, $label:literal, $effect:ident { $($(#[$attr:meta])* $field:ident : $ty:ty),* $(,)? } => $fixture:ident, $example:expr);* $(;)?) => {
        /// Registered edits, session controls and export requests. Unknown fields are refused.
        #[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(tag = "command", content = "params", deny_unknown_fields)]
        pub enum Command {
            $(#[serde(rename = $id)] $variant { $($(#[$attr])* $field: $ty),* }),*
        }
        fn declarations() -> Vec<(&'static str, &'static str, Effect, Example)> {
            vec![$(($id, $label, Effect::$effect, Example { fixture:Fixture::$fixture, params:$example })),*]
        }
        /// Parameter-free command identity, generated with its schema and executor.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum CommandKind { $($variant),* }
        impl CommandKind {
            pub const fn id(self) -> &'static str {
                match self { $(Self::$variant => $id),* }
            }
            pub fn enabled(self, session: &crate::Session) -> bool {
                registry().iter().find(|spec| spec.id == self.id())
                    .is_some_and(|spec| spec.enabled(session))
            }
        }
        impl Command {
            pub fn effect(&self) -> Effect {
                match self { $(Self::$variant { .. } => Effect::$effect),* }
            }
            pub const fn kind(&self) -> CommandKind {
                match self { $(Self::$variant { .. } => CommandKind::$variant),* }
            }
            pub fn id(&self) -> &'static str {
                self.kind().id()
            }
        }
    };
}
// A required nullable field differs from an omittable Option. This wrapper
// describes exactly the deserialize_with contract without changing owner types.
struct RequiredNullable<T>(std::marker::PhantomData<T>);
impl<T: schemars::JsonSchema> schemars::JsonSchema for RequiredNullable<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("RequiredNullable_{}", T::schema_name()).into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        generator.subschema_for::<Option<T>>()
    }
}
// Option allows explicit null, but command fields must still be present.
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
commands! {
    CreateProject, "project.create", "Create project", SessionControl { id: project::ProjectId, name: NfcText } => Create, serde_json::json!({"id":"18446744073709551615","name":"Created"});
    CreateScene, "scene.create", "Create scene", Edit { name: NfcText } => Timeline, serde_json::json!({"name":"Scene 2"});
    DeleteScenes, "scene.delete", "Delete scenes", Edit { scenes: Vec<project::SceneId> } => Timeline, serde_json::json!({"scenes":[]});
    SwitchScene, "scene.switch", "Switch scene", SessionControl { scene: project::SceneId } => Timeline, serde_json::json!({"scene":0});
    RenameScene, "scene.rename", "Rename scene", Edit { scene: project::SceneId, name: NfcText } => Timeline, serde_json::json!({"scene":0,"name":"Renamed scene"});
    RenameProject, "project.rename", "Rename project", Edit { name: NfcText } => Timeline, serde_json::json!({"name":"Renamed"});
    DuplicateProject, "project.duplicate", "Duplicate project", SessionControl { id: project::ProjectId } => Library, serde_json::json!({"id":"2"});
    DeleteProject, "project.delete", "Permanently delete project", SessionControl {} => Library, serde_json::json!({});
    ThumbnailProject, "project.thumbnail", "Refresh project thumbnail", SessionControl {} => Library, serde_json::json!({});
    SaveProject, "project.save", "Save project", SessionControl {} => Save, serde_json::json!({});
    SetMaster, "audio.set_master", "Set master audio", Edit { volume: f32, muted: bool } => Timeline, serde_json::json!({"volume":0.0,"muted":false});
    AddTransition, "transition.add", "Add video transition", Edit { track: TrackId, from: ItemId, to: ItemId, kind: String, duration: Tick, alignment: project::TransitionAlignment } => TransitionCut, serde_json::json!({"track":1,"from":10,"to":11,"kind":"cross_dissolve","duration":20,"alignment":"center"});
    MoveTransition, "transition.move", "Move video transition", Edit { transition: project::TransitionId, start: Tick } => Transition, serde_json::json!({"transition":13,"start":40});
    SetTransitionDuration, "transition.set_duration", "Resize video transition", Edit { transition: project::TransitionId, duration: Tick } => Transition, serde_json::json!({"transition":13,"duration":20});
    RemoveTransition, "transition.remove", "Remove video transition", Edit { transition: project::TransitionId } => Transition, serde_json::json!({"transition":13});
    SetFade, "audio.set_fade", "Set audio fade", Edit { item: ItemId, edge: project::FadeEdge, duration: Tick, curve: project::FadeCurve } => Audio, serde_json::json!({"item":5,"edge":"in","duration":0,"curve":"equal_power"});
    SetLoudness, "audio.set_loudness", "Set loudness target", Edit { scope: project::LoudnessScope, target_lufs: f32, true_peak_db: f32 } => Timeline, serde_json::json!({"scope":"master","target_lufs":-14.0,"true_peak_db":-1.0});
    ClearLoudness, "audio.clear_loudness", "Clear loudness target", Edit { scope: project::LoudnessScope } => Timeline, serde_json::json!({"scope":"master"});
    ApplyLoudness, "audio.apply_loudness", "Apply measured loudness gains", Edit { gains: project::LoudnessGains } => Timeline, serde_json::json!({"gains":{"key":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"clip":[],"master_db":0.0}});
    SetViewState, "project.set_view_state", "Set timeline view", SessionControl { zoom: f64, scroll: f64, playhead: Tick } => Timeline, serde_json::json!({"zoom":2.0,"scroll":10.0,"playhead":0});
    ExportSubtitles, "export_subtitles", "Export subtitle file", FilePublish { track: TrackId, format: captions::Format, languages: Vec<captions::LanguageTag>, path: std::path::PathBuf, overwrite: bool } => Timeline, serde_json::json!({"track":4,"format":"srt","languages":["en"],"path":"$output/subtitles.srt","overwrite":false});
    BurnSubtitles, "burn_subtitles", "Burn subtitles into MP4", FilePublish { range: time::TimeRange, languages: Vec<captions::LanguageTag>, style: render::CaptionStyle, portrait: bool, path: std::path::PathBuf, overwrite: bool } => Timeline, serde_json::json!({"range":{"start":0,"end":8467200000_i64},"languages":["en"],"style":{"font":"Inter","font_style":"SemiBold","size":48.0,"color":[255,255,255,255],"outline":null,"background":null,"position":"bottom","safe_area_percent":5.0,"max_lines":2},"portrait":false,"path":"$output/burn.mp4","overwrite":false});
    AddAsset, "import.add_asset", "Add asset", Edit { asset: SourceAsset } => Timeline, serde_json::json!({"asset":{"ephemeral":false,"id":99,"path":"$output/offline.mp4","content_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"probe":{"format":"mp4","duration":null,"streams":[]}}});
    RemoveAssets, "media.remove", "Remove media", Edit { assets: Vec<AssetId> } => Timeline, serde_json::json!({"assets":[98]});
    AddTrack, "timeline.add_track", "Add track", Edit { name: NfcText, kind: TrackKind, #[serde(default)] placement: TrackPlacement } => Timeline, serde_json::json!({"name":"Added","kind":"video"});
    RemoveTrack, "track.remove", "Remove track", Edit { track: TrackId } => Timeline, serde_json::json!({"track":3});
    SetTrackMuted, "track.set_muted", "Mute track", Edit { track: TrackId, muted: bool } => Timeline, serde_json::json!({"track":3,"muted":true});
    SetTrackHidden, "track.set_hidden", "Hide track", Edit { track: TrackId, hidden: bool } => Timeline, serde_json::json!({"track":3,"hidden":true});
    SetTrackMagnet, "track.set_magnet", "Main track magnet", Edit { enabled: bool } => Timeline, serde_json::json!({"enabled":false});
    SetTrackFlags, "track.set_flags", "Set track flags", Edit { track: TrackId, muted: bool, hidden: bool, locked: bool } => Timeline, serde_json::json!({"track":3,"muted":true,"hidden":false,"locked":false});
    RenameTrack, "track.rename", "Rename track", Edit { track: TrackId, name: NfcText } => Timeline, serde_json::json!({"track":3,"name":"Renamed"});
    ReorderTrack, "track.reorder", "Reorder track", Edit { track: TrackId, index: u32 } => Timeline, serde_json::json!({"track":3,"index":0});
    SetTrackLocked, "track.set_locked", "Lock track", Edit { track: TrackId, locked: bool } => Timeline, serde_json::json!({"track":3,"locked":true});
    InsertItem, "timeline.insert", "Insert item", Edit { #[serde(default)] placement: Placement, item: NewItem } => Timeline, serde_json::json!({"placement":{"mode":"auto"},"item":{"animations":{},"source_asset":null,"source_range":{"start":0,"end":8467200000_i64},"start":0,"content":{"kind":"overlay","params":{"color":[255,0,0,255]}}}});
    AddLibrarySound, "sounds.add_to_timeline", "Add library sound", Edit { #[serde(default)] placement: Placement, start: Tick, asset: SourceAsset, sound: project::LibrarySound } => Timeline, serde_json::json!({"placement":{"mode":"auto"},"start":0,"asset":{"ephemeral":false,"id":99,"path":"$output/sound.wav","content_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"probe":{"format":"wav","duration":254016000_i64,"streams":[{"index":0,"kind":"Audio","codec":"pcm_s16le","time_base":[1,8000],"duration":254016000_i64,"start":0,"frame_rate":null,"frame_count":null,"variable_frame_rate":null,"video":null,"audio":{"sample_rate":8000,"channels":1,"initial_padding":0,"trailing_padding":0}}]}},"sound":{"id":1,"name":"Whoosh","author":"Freesound user","licence":"Creative Commons 0","attribution":"Whoosh by Freesound user (Creative Commons 0)","source_url":"https://freesound.org/s/1/","audio_url":"https://cdn.freesound.org/previews/1/1-hq.mp3","downloads":10,"duration_ms":1000}});
    ImportCaptions, "captions.import", "Import ASS captions", Edit { asset: SourceAsset, language: captions::LanguageTag, start: Tick, #[serde(default)] track: Option<TrackId>, #[serde(default)] name: Option<NfcText>, cues: Vec<captions::AssCue> } => Timeline, serde_json::json!({"asset":{"ephemeral":true,"id":99,"path":"$output/captions.ass","content_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"probe":{"format":"ass","duration":254016000000_i64,"streams":[]}},"language":"en","start":0,"track":null,"name":"Captions","cues":[{"range":{"start":0,"end":254016000000_i64},"text":"Hello","style":{"font_family":"Arial","font_size_ratio":0.0555,"primary_color":[255,255,255,255],"outline_color":[0,0,0,255],"background_color":[0,0,0,0],"border_style":1,"alignment":{"numpad":2,"horizontal":"center","vertical":"bottom"},"margin_left_ratio":0.0,"margin_right_ratio":0.0,"margin_vertical_ratio":0.0,"bold":true,"italic":false,"underline":false,"strikeout":false}}]});
    MoveItem, "move_item", "Move item", Edit { item: ItemId, track: TrackId, start: Tick } => Timeline, serde_json::json!({"item":1,"track":3,"start":0});
    MoveItems, "move_items", "Move items", Edit { moves: Vec<Move> } => Timeline, serde_json::json!({"moves":[{"item":1,"track":3,"start":0}]});
    SplitAt, "split_at", "Split item", Edit { item: ItemId, at: Tick } => Timeline, serde_json::json!({"item":1,"at":8467200000_i64});
    TrimStart, "trim_start", "Trim start", Edit { item: ItemId, at: Tick } => Timeline, serde_json::json!({"item":1,"at":8467200000_i64});
    TrimEnd, "trim_end", "Trim end", Edit { item: ItemId, at: Tick } => Timeline, serde_json::json!({"item":1,"at":8467200000_i64});
    Delete, "delete", "Delete items", Edit { items: Vec<ItemId> } => Timeline, serde_json::json!({"items":[1]});
    RippleDelete, "ripple_delete", "Ripple delete", Edit { item: ItemId } => Timeline, serde_json::json!({"item":1});
    SetItemParam, "set_item_param", "Set item parameters", Edit { item: ItemId, content: ItemParams } => Timeline, serde_json::json!({"item":1,"content":{"kind":"overlay","params":{"color":[255,255,0,255]}}});
    SetItemField, "timeline.set_item_param", "Set item parameter", Edit { item: ItemId, patch: ItemParamPatch } => Timeline, serde_json::json!({"item":1,"patch":{"field":"blendMode","value":"multiply"}});
    SetCanvasBackground, "project.set_canvas_background", "Set canvas background colour", Edit { color: Color } => Timeline, serde_json::json!({"color":[0,0,0,255]});
    SetSpeed, "timeline.set_speed", "Set clip speed", Edit { items: Vec<ItemId>, speed: time::Speed, maintain_pitch: bool } => Timeline, serde_json::json!({"items":[5],"speed":2.0,"maintain_pitch":true});
    SetItemParamMany, "timeline.set_item_param_many", "Set shared item field", Edit { items: Vec<ItemId>, patch: ItemParamPatch } => Timeline, serde_json::json!({"items":[1],"patch":{"field":"text_size","value":48.0}});
    AddMask, "masks.add", "Add mask", Edit { item: ItemId, mask: project::Mask } => Timeline, serde_json::json!({"item":1,"mask":project::Mask::new("mask2",project::MaskShape::Rectangle)});
    RemoveMask, "masks.remove", "Remove mask", Edit { item: ItemId, mask: String } => Timeline, serde_json::json!({"item":1,"mask":"mask1"});
    UpdateMask, "masks.update", "Update mask", Edit { item: ItemId, mask: project::Mask } => Timeline, serde_json::json!({"item":1,"mask":project::Mask::new("mask1",project::MaskShape::Ellipse)});
    ReorderMasks, "masks.reorder", "Reorder masks", Edit { item: ItemId, order: Vec<String> } => Timeline, serde_json::json!({"item":1,"order":["mask1"]});
    AddEffect, "effects.add", "Add effect", Edit { item: ItemId, effect: project::EffectInstance } => Timeline, serde_json::json!({"item":1,"effect":{"id":"blur2","enabled":true,"params":{"kind":"blur","intensity":15.0}}});
    RemoveEffect, "effects.remove", "Remove effect", Edit { item: ItemId, effect: String } => Timeline, serde_json::json!({"item":1,"effect":"blur1"});
    UpdateEffectParam, "effects.update_param", "Update effect parameter", Edit { item: ItemId, effect: String, key: String, value: f64 } => Timeline, serde_json::json!({"item":1,"effect":"blur1","key":"intensity","value":30.0});
    ReorderEffects, "effects.reorder", "Reorder effects", Edit { item: ItemId, order: Vec<String> } => Timeline, serde_json::json!({"item":1,"order":["blur1"]});
    ToggleEffect, "effects.toggle", "Enable effect", Edit { item: ItemId, effect: String, enabled: bool } => Timeline, serde_json::json!({"item":1,"effect":"blur1","enabled":false});
    SetAnimatedParam, "keyframes.set_param", "Set animated parameter", Edit { item: ItemId, path: project::AnimationPath, param: project::Param } => Timeline, serde_json::json!({"item":1,"path":"opacity","param":{"kind":"value","channels":{"value":{"kind":"scalar","value":0.5},"keys":[],"before":"hold","after":"hold"}}});
    AddKeyframe, "keyframes.add", "Add keyframe", Edit { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::ColorComponent>")] component: Option<project::ColorComponent>, key: project::Keyframe } => Timeline, serde_json::json!({"item":1,"path":"opacity","component":null,"key":{"time":8467200000_i64,"value":{"kind":"scalar","value":0.5},"segment":"linear","tangent":"broken","incoming":null,"outgoing":null}});
    RemoveKeyframe, "keyframes.remove", "Remove keyframe", Edit { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::ColorComponent>")] component: Option<project::ColorComponent>, time: Tick } => Timeline, serde_json::json!({"item":1,"path":"opacity","component":null,"time":0});
    RetimeKeyframe, "keyframes.retime", "Move keyframe", Edit { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::ColorComponent>")] component: Option<project::ColorComponent>, from: Tick, to: Tick } => Timeline, serde_json::json!({"item":1,"path":"opacity","component":null,"from":0,"to":1});
    SetKeyframeHandle, "keyframes.set_handle", "Set keyframe handle", Edit { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::ColorComponent>")] component: Option<project::ColorComponent>, time: Tick, side: project::HandleSide, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::CurveHandle>")] handle: Option<project::CurveHandle>, tangent: project::TangentMode } => Timeline, serde_json::json!({"item":1,"path":"opacity","component":null,"time":0,"side":"out","handle":null,"tangent":"broken"});
    ScaleKeyframeTime, "keyframes.scale_time", "Stretch animation time", Edit { item: ItemId, numerator: i64, denominator: i64 } => Timeline, serde_json::json!({"item":1,"numerator":2,"denominator":1});
    Retranslate, "retranslate", "Apply selected cue translations", Edit { item: ItemId, target_language: captions::LanguageTag, texts: std::collections::BTreeMap<NfcText, NfcText>, translation_ref: NfcText, force: bool } => Timeline, serde_json::json!({"item":4,"target_language":"vi","texts":{"cue-1":"Xin chào"},"translation_ref":"example","force":true});
    AddBookmark, "add_bookmark", "Add bookmark", Edit { start: Tick, duration: Tick, note: NfcText, color: Color } => Timeline, serde_json::json!({"start":0,"duration":0,"note":"Mark","color":[255,255,0,255]});
    MoveBookmark, "bookmark.move", "Move bookmark", Edit { bookmark: BookmarkId, start: Tick } => Timeline, serde_json::json!({"bookmark":1,"start":0});
    Batch, "batch", "Batch edit", Edit { commands: Vec<Command> } => Timeline, serde_json::json!({"commands":[{"command":"timeline.add_track","params":{"name":"Batch","kind":"video"}}]});
    BeginGesture, "begin_gesture", "Begin gesture", SessionControl {} => Timeline, serde_json::json!({});
    CommitGesture, "commit_gesture", "Commit gesture", SessionControl {} => Gesture, serde_json::json!({});
    CancelGesture, "cancel_gesture", "Cancel gesture", SessionControl {} => Gesture, serde_json::json!({});
    Undo, "undo", "Undo", SessionControl {} => History, serde_json::json!({});
    Redo, "redo", "Redo", SessionControl {} => Redo, serde_json::json!({});
    Inspect, "inspect", "Inspect project", Read {} => Timeline, serde_json::json!({});
    Preview, "preview", "Preview frames", JobControl { frames: Vec<i64> } => Timeline, serde_json::json!({"frames":[0]});
    Snapshot, "preview.snapshot", "Save preview snapshot", FilePublish { time: Tick, path: std::path::PathBuf } => Timeline, serde_json::json!({"time":0,"path":"$output/snapshot.png"});
    Export, "export", "Export video", FilePublish { range: time::TimeRange, path: std::path::PathBuf, overwrite: bool } => Timeline, serde_json::json!({"range":{"start":0,"end":8467200000_i64},"path":"$output/export.mp4","overwrite":false});
    JobStatus, "job_status", "Inspect job", Read { job: crate::jobs::JobId } => Job, serde_json::json!({"job":0});
    JobCancel, "job_cancel", "Cancel job", JobControl { job: crate::jobs::JobId } => Job, serde_json::json!({"job":0});
    Catalog, "catalog", "Command catalog", Read {} => Timeline, serde_json::json!({});
    ProviderCatalog, "provider_catalog", "Provider catalog", Read {} => Timeline, serde_json::json!({});
    FontsList, "fonts.list", "List fonts", Read { #[serde(default)] query: Option<NfcText> } => Timeline, serde_json::json!({"query":"lob"});
    FontsLoad, "fonts.load", "Load font family", Read { family: NfcText } => Timeline, serde_json::json!({"family":"Lobster"});
    FontsDownload, "fonts.download", "Download font family", JobControl { family: NfcText, accept_license: bool } => Timeline, serde_json::json!({"family":"Lobster","accept_license":true});
}

pub fn registry() -> &'static [CommandSpec] {
    static REGISTRY: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut schema = schemars::schema_for!(Command).to_value();
        crate::control::close_schema(&mut schema);
        // Recursive Command refs were relative to the enum root. Each standalone
        // params document must retain that root under the same local definition.
        fn rebase(value: &mut serde_json::Value) {
            if let Some(object) = value.as_object_mut() {
                if object.get("$ref").is_some_and(|v| v == "#") {
                    object.insert("$ref".into(), serde_json::json!("#/$defs/CommandRoot"));
                }
                for v in object.values_mut() {
                    rebase(v);
                }
            } else if let Some(array) = value.as_array_mut() {
                for v in array {
                    rebase(v);
                }
            }
        }
        rebase(&mut schema);
        let mut command_root = schema.clone();
        if let Some(object) = command_root.as_object_mut() {
            object.remove("$defs");
            object.remove("$schema");
        }
        if let Some(defs) = schema
            .get_mut("$defs")
            .and_then(serde_json::Value::as_object_mut)
        {
            defs.insert("CommandRoot".into(), command_root);
        }
        let mut output = schemars::generate::SchemaSettings::draft2020_12()
            .for_serialize()
            .into_generator()
            .into_root_schema_for::<crate::control::ControlResult>()
            .to_value();
        crate::control::close_schema(&mut output);
        declarations()
            .into_iter()
            .map(|(id, label, effect, example)| {
                let mut params = schema
                    .get("oneOf")
                    .or_else(|| schema.get("anyOf"))
                    .and_then(serde_json::Value::as_array)
                    .and_then(|variants| {
                        variants.iter().find(|v| {
                            v.pointer("/properties/command/const")
                                .and_then(serde_json::Value::as_str)
                                == Some(id)
                        })
                    })
                    .and_then(|v| v.pointer("/properties/params"))
                    .cloned()
                    .unwrap_or(serde_json::json!({"type":"object", "additionalProperties":false}));
                if let Some(object) = params.as_object_mut() {
                    object.insert(
                        "$schema".into(),
                        serde_json::json!("https://json-schema.org/draft/2020-12/schema"),
                    );
                    if let Some(defs) = schema.get("$defs").and_then(serde_json::Value::as_object) {
                        object.insert("$defs".into(), reachable_defs(object, defs));
                    }
                }
                CommandSpec {
                    id,
                    label,
                    effect,
                    limits: crate::control::LIMITS,
                    params_schema: params,
                    output_schema: output.clone(),
                    example,
                }
            })
            .collect()
    })
}

/// The `$defs` a schema can reach through `#/$defs/<name>` references, so each
/// command carries its own types instead of the whole command union.
fn reachable_defs(
    root: &serde_json::Map<String, serde_json::Value>,
    defs: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    fn refs<'a>(value: &'a serde_json::Value, out: &mut Vec<&'a str>) {
        match value {
            serde_json::Value::Object(o) => {
                if let Some(name) = o
                    .get("$ref")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|r| r.strip_prefix("#/$defs/"))
                {
                    out.push(name);
                }
                o.values().for_each(|v| refs(v, out));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|v| refs(v, out)),
            _ => {}
        }
    }
    let mut keep = std::collections::BTreeSet::new();
    let mut pending = Vec::new();
    root.iter()
        .filter(|(k, _)| k.as_str() != "$defs")
        .for_each(|(_, v)| refs(v, &mut pending));
    while let Some(name) = pending.pop() {
        if let Some(def) = defs.get(name)
            && keep.insert(name)
        {
            refs(def, &mut pending);
        }
    }
    serde_json::Value::Object(
        defs.iter()
            .filter(|(k, _)| keep.contains(k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    )
}

/// Item input; the engine allocates its identity only on successful commit.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewItem {
    pub animations: project::Animations,
    pub source_asset: Option<AssetId>,
    pub source_range: SourceRange,
    pub start: Tick,
    /// Video and audio only; omitted means 1x.
    #[serde(default)]
    pub retime: project::Retime,
    pub content: ItemParams,
}
/// Absolute output destination for a simultaneous move.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Move {
    pub item: ItemId,
    pub track: TrackId,
    pub start: Tick,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CreatedId {
    Transition(project::TransitionId),
    Scene(project::SceneId),
    Track(TrackId),
    Item(ItemId),
    Bookmark(BookmarkId),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct Outcome {
    pub changed: bool,
    pub created: Vec<CreatedId>,
    pub placements: Vec<ItemPlacement>,
}
