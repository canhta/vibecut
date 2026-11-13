//! Declares the closed editing command catalog.
use super::ItemPasteSource;
use super::KeyframePasteSource;
use super::command_facts::{Availability, Facts, GrantClass, Lifecycle};
use super::transcript::{CueId, WordId};
use super::wire::OutputSpan;
use captions::NfcText;
use media::{AssetId, SourceAsset};
use project::{BookmarkId, Color, ItemId, ItemParams, TrackId, TrackKind};
use serde::{Deserialize, Serialize};
use time::{SourceRange, Tick, TimeRange};

/// Built-in opacity, transform and scale recipes expanded once into ordinary editable keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAnimationPreset {
    FadeIn,
    FadeOut,
    FadeInOut,
    SlideIn,
    SlideOut,
    SlideInOut,
    ScaleIn,
    ScaleOut,
    ScaleInOut,
}

/// Built-in looks, applied as values rather than a persisted preset identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptionPreset {
    Outline,
    Box,
}
impl CaptionPreset {
    /// Whether the complete saved style equals this look, including unused colours.
    pub fn matches(self, style: &captions::CueStyle) -> bool {
        *style == self.style()
    }
    fn style(self) -> captions::CueStyle {
        captions::CueStyle {
            font_family: "Inter".into(),
            font_size_ratio: 48.0 / 1080.0,
            primary_color: [255; 4],
            outline_color: [0, 0, 0, 255],
            background_color: if self == Self::Box {
                [0, 0, 0, 255]
            } else {
                [0; 4]
            },
            border_style: if self == Self::Box { 3 } else { 1 },
            alignment: captions::CueAlignment {
                numpad: 2,
                horizontal: captions::HorizontalAlignment::Center,
                vertical: captions::VerticalAlignment::Bottom,
            },
            margin_left_ratio: 0.05,
            margin_right_ratio: 0.05,
            margin_vertical_ratio: 0.05,
            bold: true,
            italic: false,
            underline: false,
            strikeout: false,
        }
    }
}
/// How `captions.select_emphasis` chooses keywords. Lexicon is deterministic and local.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EmphasisMode {
    #[default]
    Lexicon,
}
/// One whole-item style choice: a built-in or imported value materializes into
/// cue styles, while a saved preset stays a live project reference with a
/// sparse patch.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptionStyleApply {
    Preset {
        preset: CaptionPreset,
    },
    Custom {
        style: captions::CueStyle,
    },
    Saved {
        preset: project::CaptionPresetId,
        #[serde(default, skip_serializing_if = "captions::CaptionOverrides::is_empty")]
        overrides: captions::CaptionOverrides,
    },
}
impl CaptionStyleApply {
    /// The materialized cue value for a built-in or imported choice.
    pub(super) fn materialized(&self) -> Option<captions::CueStyle> {
        match self {
            Self::Preset { preset } => Some(preset.style()),
            Self::Custom { style } => Some(style.clone()),
            Self::Saved { .. } => None,
        }
    }
}

/// Output edge moved by a group trim; every member receives the same tick delta.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrimEdge {
    Start,
    End,
}

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
    TextBackgroundPaddingX(f32),
    TextBackgroundPaddingY(f32),
    TextBackgroundOffsetX(f32),
    TextBackgroundOffsetY(f32),
    TextBackgroundCornerRadius(f32),
    TextAlign(project::TextAlign),
    TextDecoration(project::TextDecoration),
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
    Crop(Option<project::Crop>),
    GraphicFill(Color),
    GraphicStroke(Color),
    GraphicStrokeWidth(f64),
    GraphicStrokeAlign(project::StrokeAlign),
    GraphicCornerRadius(f64),
    GraphicSides(f64),
    GraphicPoints(f64),
    GraphicDepth(f64),
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
            ) | (Self::Crop(_), ItemParams::Video(_))
                | (
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
                        | Self::TextBackgroundPaddingX(_)
                        | Self::TextBackgroundPaddingY(_)
                        | Self::TextBackgroundOffsetX(_)
                        | Self::TextBackgroundOffsetY(_)
                        | Self::TextBackgroundCornerRadius(_)
                        | Self::TextAlign(_)
                        | Self::TextDecoration(_)
                        | Self::TextPositionX(_)
                        | Self::TextPositionY(_),
                    ItemParams::Text(_)
                )
                | (
                    Self::GraphicFill(_) | Self::GraphicStroke(_),
                    ItemParams::Overlay(_) | ItemParams::Graphic(_)
                )
                | (
                    Self::GraphicStrokeWidth(_) | Self::GraphicStrokeAlign(_),
                    ItemParams::Graphic(_)
                )
                | (
                    Self::GraphicCornerRadius(_),
                    ItemParams::Graphic(
                        project::GraphicParams::Rectangle { .. }
                            | project::GraphicParams::Polygon { .. }
                    )
                )
                | (
                    Self::GraphicSides(_),
                    ItemParams::Graphic(project::GraphicParams::Polygon { .. })
                )
                | (
                    Self::GraphicPoints(_) | Self::GraphicDepth(_),
                    ItemParams::Graphic(project::GraphicParams::Star { .. })
                )
        )
    }
    pub(crate) fn validate(&self) -> crate::session::Result<()> {
        if let Some((field, value)) = self.transform() {
            return if field.clamp(value).is_ok_and(|bounded| bounded == value) {
                Ok(())
            } else {
                Err(crate::session::EditError::Invalid("transform bounds"))
            };
        }
        let valid = match self {
            Self::GraphicStrokeWidth(v) => v.is_finite() && (0. ..=64.).contains(v),
            Self::GraphicCornerRadius(v) => v.is_finite() && (0. ..=50.).contains(v),
            Self::GraphicSides(v) | Self::GraphicPoints(v) => {
                v.is_finite() && (3. ..=12.).contains(v)
            }
            Self::GraphicDepth(v) => v.is_finite() && (1. ..=99.).contains(v),
            Self::GraphicStrokeAlign(_) => true,
            Self::BlendMode(_) => true,
            Self::Opacity(v) => v.is_finite() && (0.0..=1.0).contains(v),
            Self::AudioGain(v) => project::valid_volume(*v),
            Self::AudioMuted(_) => true,
            Self::Crop(crop) => crop.as_ref().is_none_or(|crop| crop.valid()),
            Self::TextContent(v) => v.as_str().len() <= 1 << 20,
            Self::TextFont(v) => !v.as_str().trim().is_empty() && v.as_str().len() <= 256,
            Self::TextWeight(v) => project::valid_weight(*v),
            Self::TextItalic(_) => true,
            Self::TextLetterSpacing(v) => project::valid_letter_spacing(*v),
            Self::TextLineHeight(v) => project::valid_line_height(*v),
            Self::TextSize(v) => v.is_finite() && (1.0..=4096.0).contains(v),
            Self::TextColor(_)
            | Self::TextAlign(_)
            | Self::TextDecoration(_)
            | Self::TextBackgroundColor(_)
            | Self::GraphicFill(_)
            | Self::GraphicStroke(_) => true,
            Self::TextBackground(background) => background
                .as_ref()
                .is_none_or(project::TextBackground::valid),
            Self::TextBackgroundPaddingX(v)
            | Self::TextBackgroundPaddingY(v)
            | Self::TextBackgroundCornerRadius(v) => {
                v.is_finite() && (0.0..=project::TextBackground::MAX_EXTENT).contains(v)
            }
            Self::TextBackgroundOffsetX(v) | Self::TextBackgroundOffsetY(v) => {
                v.is_finite() && v.abs() <= project::TextBackground::MAX_OFFSET
            }
            Self::TextPositionX(v) | Self::TextPositionY(v) => {
                v.is_finite() && (0.0..=100.0).contains(v)
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(crate::session::EditError::Invalid("shared field value"))
        }
    }
}

/// One addressed keyframe: its item, animation path, optional colour component and time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyframeRef {
    pub item: ItemId,
    pub path: project::AnimationPath,
    pub component: Option<project::ColorComponent>,
    pub time: Tick,
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
    pub limits: super::wire::Limits,
    pub params_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub example: Example,
    /// Declaration facts below never enter the wire catalog or its digest.
    #[serde(skip)]
    pub availability: Availability,
    /// `None` only when a declaration forgot it; the registry test refuses that.
    #[serde(skip)]
    pub grant: Option<GrantClass>,
    #[serde(skip)]
    pub curated: bool,
    #[serde(skip)]
    pub lifecycle: Option<Lifecycle>,
}
/// Fixture requirements are registry data; conformance consumers never switch on IDs.
#[derive(Clone, Copy, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fixture {
    /// Uniform saved styles on caption item 4 and named bank entry 100.
    CaptionPresetBank,
    /// Timeline with Text item 3 on the copied empty Text track 3.
    Text,
    /// Text fixture plus one saved style under `TextPresetId(100)`.
    TextPresetBank,
    TransitionCut,
    Transition,
    /// Timeline with three visual items spread far enough apart to distribute.
    Arrange,
    Timeline,
    /// Timeline whose item 1 opacity carries a second key at 1 s, so its first segment is easable.
    Keyframes,
    /// Timeline whose item 1 carries one closed cubic path mask, `mask1`.
    MaskPath,
    /// Timeline plus one audible clip, item 5.
    Audio,
    /// Timeline whose item 5 is a video with an audible source asset.
    SourceAudio,
    /// Timeline plus one mono audio asset, id 97, with a real local file.
    Waveform,
    /// Timeline plus one still-image clip, item 6, with a real local file.
    Thumbnails,
    /// Timeline whose caption cue carries measured source words.
    Words,
    /// Timeline whose caption item 4 cue `cue-1` carries verified word records over
    /// "Quarter revenue grew 40 percent worldwide".
    VerifiedWords,
    /// Timeline plus one real local video asset, id 97, at least one second long.
    Video,
    /// Timeline plus one real local video asset, id 97, and video item 8 over it.
    Reframe,
    /// The `Reframe` fixture with one completed generate already applied to item 8.
    ReframeShot,
    /// Timeline plus one real audible clip, item 5, and a master loudness target.
    Loudness,
    /// Timeline plus a real source asset and its retained completed transcription.
    Captions,
    /// The `Waveform` fixture with one completed `clips.suggest` job, job 0, retained.
    Clips,
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
/// Lifecycle route of a command id: the single answer every adapter shares for
/// commands that the library or persistence owner executes.
pub fn lifecycle(id: &str) -> Option<Lifecycle> {
    CommandKind::from_id(id)?.lifecycle()
}
impl Command {
    /// The published file or directory and whether it may replace an existing one,
    /// for the commands that publish output. A new publishing command must be added
    /// here; the registry test refuses a `FilePublish` declaration whose example
    /// yields none. A directory names its own files and is refused by the output
    /// grant unless it is the granted directory.
    pub fn published_path(&self) -> Option<(std::path::PathBuf, bool)> {
        match self {
            Self::Export {
                path, overwrite, ..
            }
            | Self::ExportSubtitles {
                path, overwrite, ..
            }
            | Self::BurnSubtitles {
                path, overwrite, ..
            } => Some((path.clone(), *overwrite)),
            Self::Snapshot { path, .. } => Some((path.clone(), false)),
            Self::ClipsExport {
                directory,
                overwrite,
                ..
            } => Some((directory.clone(), *overwrite)),
            Self::ChaptersExportSidecars { dir, .. } => Some((dir.clone(), false)),
            Self::ExportClips { dir, overwrite, .. } => Some((dir.clone(), *overwrite)),
            _ => None,
        }
    }
}
impl CommandSpec {
    /// Coarse session availability. Target/parameter validation occurs at execution.
    pub fn enabled(&self, session: &crate::Session) -> bool {
        self.availability.holds(session)
    }
}
// Collect only edit variants and move their payloads from the public command.
// Each effect is classified explicitly: a new effect must choose its admission rule.
macro_rules! edit_commands {
    (@collect [$($variants:tt)*] [$($arms:tt)*];) => {
        pub(super) enum EditCommand { $($variants)* }
        impl Command {
            #[allow(
                clippy::result_large_err,
                reason = "the refusal returns the original public command so callers can keep it; boxing would change the API"
            )]
            pub(super) fn into_edit(self) -> Result<EditCommand, Self> {
                match self { $($arms)* }
            }
        }
    };
    (@collect [$($variants:tt)*] [$($arms:tt)*];
        $first:ident Edit { $($a:ident: $at:ty),* };
        $second:ident Edit { $($b:ident: $bt:ty),* }; $($rest:tt)*) => {
        edit_commands!(@collect
            [$($variants)* $first { $($a: $at),* }, $second { $($b: $bt),* },]
            [$($arms)* Self::$first { $($a),* } => Ok(EditCommand::$first { $($a),* }),
                Self::$second { $($b),* } => Ok(EditCommand::$second { $($b),* }),];
            $($rest)*);
    };
    (@collect [$($variants:tt)*] [$($arms:tt)*];
        $variant:ident Edit { $($field:ident: $ty:ty),* }; $($rest:tt)*) => {
        edit_commands!(@collect
            [$($variants)* $variant { $($field: $ty),* },]
            [$($arms)* Self::$variant { $($field),* } => Ok(EditCommand::$variant { $($field),* }),];
            $($rest)*);
    };
    (@collect [$($variants:tt)*] [$($arms:tt)*];
        $variant:ident Read { $($fields:tt)* }; $($rest:tt)*) => {
        edit_commands!(@collect [$($variants)*]
            [$($arms)* command @ Self::$variant { .. } => Err(command),]; $($rest)*);
    };
    (@collect [$($variants:tt)*] [$($arms:tt)*];
        $variant:ident SessionControl { $($fields:tt)* }; $($rest:tt)*) => {
        edit_commands!(@collect [$($variants)*]
            [$($arms)* command @ Self::$variant { .. } => Err(command),]; $($rest)*);
    };
    (@collect [$($variants:tt)*] [$($arms:tt)*];
        $variant:ident FilePublish { $($fields:tt)* }; $($rest:tt)*) => {
        edit_commands!(@collect [$($variants)*]
            [$($arms)* command @ Self::$variant { .. } => Err(command),]; $($rest)*);
    };
    (@collect [$($variants:tt)*] [$($arms:tt)*];
        $variant:ident JobControl { $($fields:tt)* }; $($rest:tt)*) => {
        edit_commands!(@collect [$($variants)*]
            [$($arms)* command @ Self::$variant { .. } => Err(command),]; $($rest)*);
    };
}
/// One line per command: `Variant, "id", "Label", Effect { params } => Fixture, example`
/// then, in this order and each optional: `, available = Availability`,
/// `, grant = GrantClass` (defaults by effect, required for session and job control),
/// `, curated = true`, `, lifecycle = Lifecycle`.
macro_rules! commands {
    ($($variant:ident, $id:literal, $label:literal, $effect:ident { $($(#[$attr:meta])* $field:ident : $ty:ty),* $(,)? } => $fixture:ident, $example:expr
        $(, available = $available:ident)? $(, grant = $grant:ident)? $(, curated = $curated:literal)? $(, lifecycle = $lifecycle:ident)?);* $(;)?) => {
        /// Registered edits, session controls and export requests. Unknown fields are refused.
        #[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(tag = "command", content = "params", deny_unknown_fields)]
        pub enum Command {
            $(#[serde(rename = $id)] $variant { $($(#[$attr])* $field: $ty),* }),*
        }
        // Field attributes belong to public serde/schema admission, not the private reducer.
        edit_commands!(@collect [] []; $($variant $effect { $($field: $ty),* };)*);
        fn declarations() -> Vec<(&'static str, &'static str, Effect, Example, Facts)> {
            vec![$(($id, $label, Effect::$effect, Example { fixture:Fixture::$fixture, params:$example }, CommandKind::$variant.facts())),*]
        }
        /// Parameter-free command identity, generated with its schema and executor.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum CommandKind { $($variant),* }
        impl CommandKind {
            pub const fn id(self) -> &'static str {
                match self { $(Self::$variant => $id),* }
            }
            pub fn from_id(id: &str) -> Option<Self> {
                match id { $($id => Some(Self::$variant),)* _ => None }
            }
            pub fn enabled(self, session: &crate::Session) -> bool {
                self.facts().available.holds(session)
            }
            /// Grant class of this command; `None` only if its declaration omitted a
            /// required one, which the registry test refuses.
            pub fn grant(self) -> Option<GrantClass> {
                self.facts().grant
            }
            pub fn lifecycle(self) -> Option<Lifecycle> {
                self.facts().lifecycle
            }
            pub(super) fn facts(self) -> Facts {
                match self {
                    $(Self::$variant => Facts {
                        available: Option::<Availability>::None $(.or(Some(Availability::$available)))?.unwrap_or(Availability::Always),
                        grant: Option::<GrantClass>::None $(.or(Some(GrantClass::$grant)))?.or(Effect::$effect.default_grant()),
                        curated: false $(|| $curated)?,
                        lifecycle: Option::<Lifecycle>::None $(.or(Some(Lifecycle::$lifecycle)))?,
                    }),*
                }
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
            pub fn grant(&self) -> Option<GrantClass> {
                self.kind().grant()
            }
            pub fn lifecycle(&self) -> Option<Lifecycle> {
                self.kind().lifecycle()
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
    AddCleanupEffect, "audio.add_cleanup_effect", "Add audio cleanup effect", Edit { item: ItemId, effect: project::CleanupEffect } => Audio, serde_json::json!({"item":5,"effect":{"id":1,"enabled":true,"wet_percent":100.0,"kind":{"kind":"hum","mains":"hz50","harmonics":5,"q":30.0}}}), available = UnlockedItem;
    ApplyLoudness, "audio.apply_loudness", "Apply measured loudness gains", Edit { gains: project::LoudnessGains } => Timeline, serde_json::json!({"gains":{"key":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"clip":[],"master_db":0.0}});
    ClearLoudness, "audio.clear_loudness", "Clear loudness target", Edit { scope: project::LoudnessScope } => Timeline, serde_json::json!({"scope":"master"});
    MeasureLoudness, "audio.measure_loudness", "Measure loudness", JobControl {} => Loudness, serde_json::json!({}), grant = Agent;
    RemoveCleanupEffect, "audio.remove_cleanup_effect", "Remove audio cleanup effect", Edit { item: ItemId, effect: u32 } => Audio, serde_json::json!({"item":5,"effect":1}), available = UnlockedItem;
    SetFade, "audio.set_fade", "Set audio fade", Edit { item: ItemId, edge: project::FadeEdge, duration: Tick, curve: project::FadeCurve } => Audio, serde_json::json!({"item":5,"edge":"in","duration":0,"curve":"equal_power"}), available = UnlockedItem;
    SetLoudness, "audio.set_loudness", "Set loudness target", Edit { scope: project::LoudnessScope, target_lufs: f32, true_peak_db: f32 } => Timeline, serde_json::json!({"scope":"master","target_lufs":-14.0,"true_peak_db":-1.0});
    SetMaster, "audio.set_master", "Set master audio", Edit { volume: f32, muted: bool } => Timeline, serde_json::json!({"volume":0.0,"muted":false});
    UpdateCleanupEffect, "audio.update_cleanup_effect", "Update audio cleanup effect", Edit { item: ItemId, effect: project::CleanupEffect } => Audio, serde_json::json!({"item":5,"effect":{"id":1,"enabled":false,"wet_percent":50.0,"kind":{"kind":"bass","boost_db":3.0}}}), available = UnlockedItem;
    Batch, "batch", "Batch edit", Edit { commands: Vec<Command> } => Timeline, serde_json::json!({"commands":[{"command":"timeline.add_track","params":{"name":"Batch","kind":"video"}}]}), curated = true;
    BeginGesture, "begin_gesture", "Begin gesture", SessionControl {} => Timeline, serde_json::json!({}), available = NoTransaction, grant = Human;
    BookmarkAdd, "bookmark.add", "Add bookmark", Edit { start: Tick, duration: Tick, note: NfcText, color: Color } => Timeline, serde_json::json!({"start":0,"duration":0,"note":"Mark","color":[255,255,0,255]});
    MoveBookmark, "bookmark.move", "Move bookmark", Edit { bookmark: BookmarkId, start: Tick } => Timeline, serde_json::json!({"bookmark":1,"start":0});
    RemoveBookmark, "bookmark.remove", "Remove bookmark", Edit { bookmark: BookmarkId } => Timeline, serde_json::json!({"bookmark":1});
    ToggleBookmark, "bookmark.toggle", "Toggle bookmark", Edit { at: Tick } => Timeline, serde_json::json!({"at":0});
    UpdateBookmark, "bookmark.update", "Update bookmark", Edit { bookmark: BookmarkId, #[serde(default)] note: Option<NfcText>, #[serde(default)] color: Option<Color>, #[serde(default)] duration: Option<Tick> } => Timeline, serde_json::json!({"bookmark":1,"note":"Marked","color":[0,157,255,255],"duration":0});
    BrandList, "brand.list", "List active brand kit", Read {} => Timeline, serde_json::json!({});
    BurnSubtitles, "burn_subtitles","Burn subtitles into MP4", FilePublish { range: time::TimeRange, languages: Vec<captions::LanguageTag>, style: render::CaptionStyle, portrait: bool, path: std::path::PathBuf, overwrite: bool } => Timeline, serde_json::json!({"range":{"start":0,"end":8467200000_i64},"languages":["en"],"style":{"font":"Inter","font_style":"SemiBold","size":48.0,"color":[255,255,255,255],"outline":null,"background":null,"position":"bottom","safe_area_percent":5.0,"max_lines":2},"portrait":false,"path":"$output/burn.mp4","overwrite":false}), available = CaptionItem;
    CancelGesture, "cancel_gesture", "Cancel gesture", SessionControl {} => Gesture, serde_json::json!({}), available = GestureActive, grant = Human;
    CaptionsApply, "captions.apply", "Apply generated captions", Edit { job: super::wire::JobId, #[serde(default)] start: Option<Tick> } => Captions, serde_json::json!({"job":0});
    ApplyCaptionStyle, "captions.apply_style", "Apply caption style", Edit { item: ItemId, style: CaptionStyleApply } => Timeline, serde_json::json!({"item":4,"style":{"kind":"preset","preset":"outline"}}), available = CaptionItem, grant = Agent;
    DeleteCaptionPreset, "captions.delete_preset", "Delete caption preset", Edit { preset: project::CaptionPresetId } => CaptionPresetBank, serde_json::json!({"preset":100}), available = NoTransaction, grant = Agent;
    CaptionsGenerate, "captions.generate", "Generate captions", JobControl { asset: media::AssetId, #[serde(default)] range: Option<SourceRange>, #[serde(default)] source_language: Option<captions::LanguageTag> } => Waveform, serde_json::json!({"asset":97,"source_language":"en"}), grant = Agent;
    ImportCaptions, "captions.import", "Import ASS captions", Edit { asset: SourceAsset, language: captions::LanguageTag, start: Tick, #[serde(default)] track: Option<TrackId>, #[serde(default)] name: Option<NfcText>, styles: Vec<captions::CueStyle>, cues: Vec<captions::AssCue> } => Timeline, serde_json::json!({"asset":{"ephemeral":true,"id":99,"path":"$output/captions.ass","content_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"probe":{"format":"ass","duration":254016000000_i64,"streams":[]}},"language":"en","start":0,"track":null,"name":"Captions","styles":[{"font_family":"Arial","font_size_ratio":0.0555,"primary_color":[255,255,255,255],"outline_color":[0,0,0,255],"background_color":[0,0,0,0],"border_style":1,"alignment":{"numpad":2,"horizontal":"center","vertical":"bottom"},"margin_left_ratio":0.0,"margin_right_ratio":0.0,"margin_vertical_ratio":0.0,"bold":true,"italic":false,"underline":false,"strikeout":false}],"cues":[{"range":{"start":0,"end":254016000000_i64},"text":"Hello","style":0}]}), grant = Human;
    RenameCaptionPreset, "captions.rename_preset", "Rename caption preset", Edit { preset: project::CaptionPresetId, name: NfcText } => CaptionPresetBank, serde_json::json!({"preset":100,"name":"Renamed look"}), available = NoTransaction, grant = Agent;
    SaveCaptionPreset, "captions.save_preset", "Save caption preset", Edit { item: ItemId, name: NfcText } => CaptionPresetBank, serde_json::json!({"item":4,"name":"Saved look"}), available = CaptionItem, grant = Agent;
    SelectCaptionEmphasis, "captions.select_emphasis", "Select caption keyword emphasis", Edit { item: ItemId, cue_ids: Vec<NfcText>, #[serde(default)] mode: EmphasisMode } => VerifiedWords, serde_json::json!({"item":4,"cue_ids":["cue-1"],"mode":"lexicon"}), available = CaptionItem, grant = Agent;
    SetCaptionEmphasis, "captions.set_emphasis", "Set caption emphasis words", Edit { item: ItemId, cue_id: NfcText, word_ids: Vec<u32> } => VerifiedWords, serde_json::json!({"item":4,"cue_id":"cue-1","word_ids":[3]}), available = CaptionItem, grant = Agent;
    SetCaptionWordHighlight, "captions.set_word_highlight", "Highlight caption words", Edit { item: ItemId, enabled: bool } => Timeline, serde_json::json!({"item":4,"enabled":true}), available = CaptionItem, grant = Agent;
    CaptionsSuggestEmphasis, "captions.suggest_emphasis", "Suggest caption emphasis with the lent provider", JobControl { item: ItemId, #[serde(default)] cue_ids: Option<Vec<NfcText>> } => VerifiedWords, serde_json::json!({"item":4}), available = CaptionItem, grant = Agent;
    CaptionsTranslate, "captions.translate", "Translate captions", JobControl { item: ItemId, #[serde(default)] cues: Option<Vec<NfcText>>, target_language: captions::LanguageTag, #[serde(default)] force: bool, #[serde(default)] #[schemars(length(max = 128))] glossary: Vec<ai::GlossaryTerm> } => Timeline, serde_json::json!({"item":4,"target_language":"vi"}), grant = Agent;
    Catalog, "catalog", "Command catalog", Read {} => Timeline, serde_json::json!({});
    ChaptersExportSidecars, "chapters.export_sidecars", "Export chapter sidecars", FilePublish { dir: std::path::PathBuf, document: ai::text::chapters::ChapterDocument } => Words, serde_json::json!({"dir":"$output","document":{"schema_version":1,"language":"en","duration":254016000000_i64,"title":"Opening","description":"A conversation about the work.","chapters":[{"start":0,"segment_id":"4:cue-1","title":"Opening"}]}});
    ChaptersGenerate, "chapters.generate", "Generate chapters", JobControl { language: captions::LanguageTag, max_chapters: usize } => Words, serde_json::json!({"language":"en","max_chapters":10}), grant = Agent;
    CopyItems, "clipboard.copy", "Copy selected items", SessionControl { #[schemars(length(max = 1024))] items: Vec<ItemId> } => Timeline, serde_json::json!({"items":[1]}), available = NoTransaction, grant = Agent;
    CopyKeyframes, "clipboard.copy_keyframes", "Copy selected keyframes", SessionControl { #[schemars(length(max = 4096))] keys: Vec<KeyframeRef> } => Timeline, serde_json::json!({"keys":[{"item":1,"path":"opacity","component":null,"time":0}]}), available = NoTransaction, grant = Agent;
    PasteItems, "clipboard.paste", "Paste items", Edit { at: Tick, source: ItemPasteSource } => Timeline, serde_json::json!({"at":0,"source":{"kind":"payload","payload":{"version":1,"span":1000,"items":[{"trackKind":"video","trackIndex":0,"item":{"blendMode":"normal","masks":[],"effects":[],"animations":{},"id":0,"hidden":false,"source_asset":null,"source_range":{"start":0,"end":1000},"start":0,"retime":{"speed":1.0,"maintain_pitch":true},"content":{"kind":"overlay","params":{"color":[255,0,0,255]}}}}]}}}), available = NoTransaction, grant = Agent;
    PasteKeyframes, "clipboard.paste_keyframes", "Paste keyframes", Edit { item: ItemId, at: Tick, source: KeyframePasteSource } => Timeline, serde_json::json!({"item":1,"at":12,"source":{"kind":"payload","payload":{"version":1,"origin":0,"params":[{"path":"opacity","param":{"kind":"value","channels":{"value":{"kind":"scalar","value":0.7},"keys":[{"time":0,"value":{"kind":"scalar","value":0.25},"segment":"linear","tangent":"flat","incoming":null,"outgoing":null}],"before":"hold","after":"hold"}}}]}}}), available = UnlockedItem, grant = Agent;
    ReadClipboard, "clipboard.read", "Read keyframe clipboard", Read {} => Timeline, serde_json::json!({}), available = NoTransaction, grant = Agent;
    ClipsExport, "clips.export", "Export clip candidates", FilePublish { job: super::wire::JobId, select: Vec<usize>, #[serde(default)] item: Option<ItemId>, #[serde(default)] languages: Vec<captions::LanguageTag>, #[serde(default)] formats: Vec<captions::Format>, directory: std::path::PathBuf, overwrite: bool, #[serde(default)] allow_degraded: bool } => Clips, serde_json::json!({"job":0,"select":[0],"directory":"$output","overwrite":false});
    ClipsSuggest, "clips.suggest", "Suggest clips", JobControl { asset: media::AssetId, #[serde(default)] source_language: Option<captions::LanguageTag>, #[serde(default)] provider: Option<String>, #[serde(default)] model: Option<String>, #[serde(default)] count: Option<usize>, #[serde(default)] min_duration: Option<Tick>, #[serde(default)] max_duration: Option<Tick>, #[serde(default)] max_segments: Option<usize>, #[serde(default)] min_segment_duration: Option<Tick>, #[serde(default)] overlap_tolerance: Option<f64>, #[serde(default)] keywords: Vec<NfcText> } => Waveform, serde_json::json!({"asset":97,"source_language":"en"}), grant = Agent;
    ImportLut, "color.import_lut", "Import 3D cube LUT", JobControl { path:std::path::PathBuf, #[serde(default)] name:Option<NfcText>, #[serde(default)] licence:Option<NfcText>, #[serde(default)] expected_hash:Option<project::LutId> } => Timeline, serde_json::json!({"path":"$output/look.cube","name":null,"licence":null,"expected_hash":null}), available = NoTransaction, grant = Human;
    RegisterLut, "color.register_lut", "Register imported LUT", Edit { record:project::LutRecord } => Timeline, serde_json::json!({"record":{"id":"0000000000000000000000000000000000000000000000000000000000000000","name":"Look","licence":null,"source_name":"look.cube","source_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"directory":"$output/lut"}}), grant = Human;
    CommitGesture, "commit_gesture", "Commit gesture", SessionControl {} => Gesture, serde_json::json!({}), available = GestureActive, grant = Human;
    Delete, "delete", "Delete items", Edit { items: Vec<ItemId> } => Timeline, serde_json::json!({"items":[1]}), available = UnlockedItem;
    AddEffect, "effects.add", "Add effect", Edit { item: ItemId, effect: project::EffectInstance } => Timeline, serde_json::json!({"item":1,"effect":{"id":"blur2","kind":"blur","enabled":true,"params":{"intensity":{"kind":"scalar","value":15.0}}}}), available = VisualItem;
    RemoveEffect, "effects.remove", "Remove effect", Edit { item: ItemId, effect: String } => Timeline, serde_json::json!({"item":1,"effect":"blur1"}), available = VisualItem;
    ReorderEffects, "effects.reorder", "Reorder effects", Edit { item: ItemId, order: Vec<String> } => Timeline, serde_json::json!({"item":1,"order":["blur1"]}), available = VisualItem;
    ToggleEffect, "effects.toggle", "Enable effect", Edit { item: ItemId, effect: String, enabled: bool } => Timeline, serde_json::json!({"item":1,"effect":"blur1","enabled":false}), available = VisualItem;
    UpdateEffectParam, "effects.update_param", "Update effect parameter", Edit { item: ItemId, effect: String, key: String, value: project::EffectParamUpdate } => Timeline, serde_json::json!({"item":1,"effect":"blur1","key":"intensity","value":30.0}), available = VisualItem;
    Export, "export", "Export video", FilePublish { settings: render::OutputOptions, path: std::path::PathBuf, overwrite: bool } => Timeline, serde_json::json!({"settings":render::OutputOptions{range:render::RangeChoice::InOut{start:time::Tick::ZERO,end:time::Tick(8467200000_i64)},..render::OutputOptions::default()},"path":"$output/export.mp4","overwrite":false}), curated = true;
    ExportClips, "export.clips", "Export clips", FilePublish { #[schemars(length(max = 256))] asset_ids: Vec<media::AssetId>, dir: std::path::PathBuf, overwrite: bool } => Video, serde_json::json!({"asset_ids":[97],"dir":"$output","overwrite":false}), available = Assets;
    ExportSubtitles, "export_subtitles", "Export subtitle file", FilePublish { track: TrackId, format: captions::Format, languages: Vec<captions::LanguageTag>, path: std::path::PathBuf, overwrite: bool } => Timeline, serde_json::json!({"track":4,"format":"srt","languages":["en"],"path":"$output/subtitles.srt","overwrite":false}), available = CaptionItem;
    FillersClassify, "fillers.classify", "Classify fillers with the lent provider", JobControl { language: captions::LanguageTag, include_markers: bool } => Words, serde_json::json!({"language":"en","include_markers":false}), available = CaptionItem, grant = Agent;
    FontsDownload, "fonts.download", "Download font family", JobControl { family: NfcText, accept_license: bool } => Timeline, serde_json::json!({"family":"Lobster","accept_license":true}), grant = Human;
    FontsImport, "fonts.import", "Import custom font", JobControl { path: std::path::PathBuf, accept_license: bool } => Timeline, serde_json::json!({"path":"$output/font.ttf","accept_license":true}), grant = Human;
    FontsList, "fonts.list", "List fonts", Read { #[serde(default)] query: Option<NfcText> } => Timeline, serde_json::json!({"query":"lob"});
    FontsLoad, "fonts.load", "Load font family", Read { family: NfcText } => Timeline, serde_json::json!({"family":"Lobster"});
    AddAsset, "import.add_asset", "Add asset", Edit { asset: SourceAsset } => Timeline, serde_json::json!({"asset":{"ephemeral":false,"id":99,"path":"$output/offline.mp4","content_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"probe":{"format":"mp4","duration":null,"streams":[]}}}), grant = Human;
    Inspect, "inspect", "Inspect project", Read {} => Timeline, serde_json::json!({}), curated = true;
    JobCancel, "job_cancel", "Cancel job", JobControl { job: super::wire::JobId } => Job, serde_json::json!({"job":0}), grant = Agent, curated = true;
    JobStatus, "job_status", "Inspect job", Read { job: super::wire::JobId } => Job, serde_json::json!({"job":0}), curated = true;
    AddKeyframe, "keyframes.add", "Add keyframe", Edit { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::ColorComponent>")] component: Option<project::ColorComponent>, key: project::Keyframe } => Timeline, serde_json::json!({"item":1,"path":"opacity","component":null,"key":{"time":8467200000_i64,"value":{"kind":"scalar","value":0.5},"segment":"linear","tangent":"broken","incoming":null,"outgoing":null}}), available = UnlockedItem;
    ApplyEasingPreset, "keyframes.apply_preset", "Apply easing preset", Edit { keys: Vec<KeyframeRef>, preset: project::EasingPreset } => Keyframes, serde_json::json!({"keys":[{"item":1,"path":"opacity","component":null,"time":0}],"preset":"ease_out"}), available = UnlockedItem;
    DeleteKeyframes, "keyframes.delete_selection", "Delete selected keyframes", Edit { keys: Vec<KeyframeRef> } => Timeline, serde_json::json!({"keys":[{"item":1,"path":"opacity","component":null,"time":0}]}), available = UnlockedItem;
    RemoveKeyframe, "keyframes.remove", "Remove keyframe", Edit { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::ColorComponent>")] component: Option<project::ColorComponent>, time: Tick } => Timeline, serde_json::json!({"item":1,"path":"opacity","component":null,"time":0}), available = UnlockedItem;
    RetimeKeyframe, "keyframes.retime", "Move keyframe", Edit { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::ColorComponent>")] component: Option<project::ColorComponent>, from: Tick, to: Tick } => Timeline, serde_json::json!({"item":1,"path":"opacity","component":null,"from":0,"to":1}), available = UnlockedItem;
    ScaleKeyframeTime, "keyframes.scale_time", "Stretch animation time", Edit { item: ItemId, numerator: i64, denominator: i64 } => Timeline, serde_json::json!({"item":1,"numerator":2,"denominator":1}), available = UnlockedItem;
    SetKeyframeExtrapolation, "keyframes.set_extrapolation", "Set keyframe extrapolation", Edit { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::ColorComponent>")] component: Option<project::ColorComponent>, before: project::Extrapolation, after: project::Extrapolation } => Timeline, serde_json::json!({"item":1,"path":"opacity","component":null,"before":"hold","after":"linear"}), available = UnlockedItem;
    SetKeyframeHandle, "keyframes.set_handle", "Set keyframe handle", Edit { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::ColorComponent>")] component: Option<project::ColorComponent>, time: Tick, side: project::HandleSide, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::CurveHandle>")] handle: Option<project::CurveHandle>, tangent: project::TangentMode } => Timeline, serde_json::json!({"item":1,"path":"opacity","component":null,"time":0,"side":"out","handle":null,"tangent":"broken"}), available = UnlockedItem;
    SetKeyframeCurve, "keyframes.set_handles", "Set keyframe curve", Edit { keys: Vec<KeyframeRef>, curve: project::EasingCurve } => Keyframes, serde_json::json!({"keys":[{"item":1,"path":"opacity","component":null,"time":0}],"curve":[0.25,0.1,0.25,1.0]}), available = UnlockedItem;
    SetAnimatedParam, "keyframes.set_param", "Set animated parameter", Edit { item: ItemId, path: project::AnimationPath, param: project::Param } => Timeline, serde_json::json!({"item":1,"path":"opacity","param":{"kind":"value","channels":{"value":{"kind":"scalar","value":0.5},"keys":[],"before":"hold","after":"hold"}}}), available = UnlockedItem;
    ToggleKeyframe, "keyframes.toggle_at", "Toggle keyframe at time", Edit { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] #[schemars(with = "RequiredNullable<project::ColorComponent>")] component: Option<project::ColorComponent>, time: Tick, value: project::ParamValue } => Timeline, serde_json::json!({"item":1,"path":"opacity","component":null,"time":8467200000_i64,"value":{"kind":"scalar","value":0.5}}), available = UnlockedItem;
    AddMask, "masks.add", "Add mask", Edit { item: ItemId, mask: project::Mask } => Timeline, serde_json::json!({"item":1,"mask":project::Mask::new("mask2",project::MaskShape::Rectangle)}), available = VisualItem;
    AddMaskPoint, "masks.add_point", "Add mask point", Edit { item: ItemId, mask: String, segment: usize, t: f64 } => MaskPath, serde_json::json!({"item":1,"mask":"mask1","segment":0,"t":0.5}), available = VisualItem;
    RemoveMask, "masks.remove", "Remove mask", Edit { item: ItemId, mask: String } => Timeline, serde_json::json!({"item":1,"mask":"mask1"}), available = VisualItem;
    RemoveMaskPoint, "masks.remove_point", "Remove mask points", Edit { item: ItemId, mask: String, points: Vec<usize> } => MaskPath, serde_json::json!({"item":1,"mask":"mask1","points":[0]}), available = VisualItem;
    ReorderMasks, "masks.reorder", "Reorder masks", Edit { item: ItemId, order: Vec<String> } => Timeline, serde_json::json!({"item":1,"order":["mask1"]}), available = VisualItem;
    SetMaskPath, "masks.set_path", "Set mask path", Edit { item: ItemId, mask: String, path: project::MaskPath } => Timeline, serde_json::json!({"item":1,"mask":"mask1","path":{"closed":true,"vertices":[{"p":[0.5,0.0],"t_in":[-0.2761423749153967,0.0],"t_out":[0.2761423749153967,0.0]},{"p":[1.0,0.5],"t_in":[0.0,-0.2761423749153967],"t_out":[0.0,0.2761423749153967]},{"p":[0.5,1.0],"t_in":[0.2761423749153967,0.0],"t_out":[-0.2761423749153967,0.0]},{"p":[0.0,0.5],"t_in":[0.0,0.2761423749153967],"t_out":[0.0,-0.2761423749153967]}]}}), available = VisualItem;
    UpdateMask, "masks.update", "Update mask", Edit { item: ItemId, mask: project::Mask } => Timeline, serde_json::json!({"item":1,"mask":project::Mask::new("mask1",project::MaskShape::Ellipse)}), available = VisualItem;
    DetectScenes, "media.detect_scenes", "Detect scene changes", JobControl { asset: AssetId, #[serde(default)] settings: crate::scene_detection::DetectionSettings } => Video, serde_json::json!({"asset":97,"settings":{"hard_threshold":0.28,"fade_threshold":0.2,"fade_step":0.005,"min_scene":127008000000_i64}}), grant = Agent;
    GenerateClipThumbnails, "media.generate_clip_thumbnails", "Generate clip thumbnails", JobControl { item: ItemId, visible: TimeRange, pixels_per_second: f64 } => Thumbnails, serde_json::json!({"item":6,"visible":{"start":0,"end":254016000000_i64},"pixels_per_second":71.5}), grant = Agent;
    GenerateWaveform, "media.generate_waveform", "Generate source waveform", JobControl { asset: AssetId } => Waveform, serde_json::json!({"asset":97}), grant = Agent;
    RemoveAssets, "media.remove", "Remove media", Edit { assets: Vec<AssetId> } => Timeline, serde_json::json!({"assets":[98]}), available = Assets;
    MoveItem, "move_item", "Move item", Edit { item: ItemId, track: TrackId, start: Tick } => Timeline, serde_json::json!({"item":1,"track":3,"start":0}), available = UnlockedItem;
    MoveItems, "move_items", "Move items", Edit { moves: Vec<Move> } => Timeline, serde_json::json!({"moves":[{"item":1,"track":3,"start":0}]}), available = UnlockedItem;
    AddLibraryMusic, "music.add_to_timeline", "Add library music", Edit { #[serde(default)] placement: Placement, start: Tick, asset: SourceAsset, music: project::LibraryMusic } => Timeline, serde_json::json!({"placement":{"mode":"auto"},"start":0,"asset":{"ephemeral":false,"id":99,"path":"$output/music.ogg","content_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"probe":{"format":"ogg","duration":254016000_i64,"streams":[{"index":0,"kind":"Audio","codec":"vorbis","time_base":[1,8000],"duration":254016000_i64,"start":0,"frame_rate":null,"frame_count":null,"variable_frame_rate":null,"video":null,"audio":{"sample_rate":8000,"channels":1,"initial_padding":0,"trailing_padding":0}}]}},"music":{"id":"0f6c1c7e-2b3a-4a9e-9d3d-2a6f1f0b8c11","title":"Night Walk","author":"Example Artist","licence":"CC BY 4.0","licence_url":"https://creativecommons.org/licenses/by/4.0/","attribution":"\"Night Walk\" by Example Artist is licensed under CC BY 4.0.","source_url":"https://commons.example/wiki/File:Night_Walk.ogg","audio_url":"https://upload.example/Night_Walk.ogg","duration_ms":31000}}), grant = Human;
    MusicSave, "music.save", "Save library music", Read { music: super::wire::MusicEntry } => Timeline, serde_json::json!({"music":{"id":"0f6c1c7e-2b3a-4a9e-9d3d-2a6f1f0b8c11","title":"Night Walk","author":"Example Artist","licence":"CC BY 4.0","licence_url":"https://creativecommons.org/licenses/by/4.0/","attribution":"\"Night Walk\" by Example Artist is licensed under CC BY 4.0.","source_url":"https://commons.example/wiki/File:Night_Walk.ogg","audio_url":"https://upload.example/Night_Walk.ogg","duration_ms":31000}}), grant = Human;
    MusicSearch, "music.search", "Search music library", Read { #[serde(default)] query: String, #[serde(default)] tag: Option<String>, page: u32, page_size: u32, #[serde(default)] saved: bool } => Timeline, serde_json::json!({"query":"night","tag":null,"page":1,"page_size":15,"saved":false}), grant = Human;
    MusicUnsave, "music.unsave", "Unsave library music", Read { id: String } => Timeline, serde_json::json!({"id":"0f6c1c7e-2b3a-4a9e-9d3d-2a6f1f0b8c11"}), grant = Human;
    Preview, "preview", "Preview frames", JobControl { frames: Vec<i64> } => Timeline, serde_json::json!({"frames":[0]}), grant = Agent, curated = true;
    Snapshot, "preview.snapshot", "Save preview snapshot", FilePublish { time: Tick, path: std::path::PathBuf } => Timeline, serde_json::json!({"time":0,"path":"$output/snapshot.png"}), available = PlayableTimeline;
    CreateProject, "project.create", "Create project", SessionControl { id: project::ProjectId, name: NfcText } => Create, serde_json::json!({"id":"18446744073709551615","name":"Created"}), grant = Human, lifecycle = Create;
    DeleteProject, "project.delete", "Move project to trash", SessionControl {} => Library, serde_json::json!({}), grant = Human, lifecycle = Delete;
    DuplicateProject, "project.duplicate", "Duplicate project", SessionControl { id: project::ProjectId } => Library, serde_json::json!({"id":"2"}), grant = Human, lifecycle = Duplicate;
    PurgeProject, "project.purge", "Permanently delete trashed project", SessionControl {} => Library, serde_json::json!({}), grant = Human, lifecycle = Purge;
    RenameProject, "project.rename", "Rename project", Edit { name: NfcText } => Timeline, serde_json::json!({"name":"Renamed"});
    RestoreProject, "project.restore", "Restore project from trash", SessionControl {} => Library, serde_json::json!({}), grant = Human, lifecycle = Restore;
    SaveProject, "project.save", "Save project", SessionControl {} => Save, serde_json::json!({}), grant = Agent, lifecycle = Save;
    SetBackground, "project.set_background", "Set canvas background colour", Edit { color: Color } => Timeline, serde_json::json!({"color":[0,0,0,255]});
    SetCanvas, "project.set_canvas", "Set canvas size", Edit { width: u32, height: u32, #[serde(default)] preset: Option<project::AspectPreset> } => Timeline, serde_json::json!({"width":1080,"height":1920,"preset":"9:16"});
    SetFps, "project.set_fps", "Set canvas frame rate", Edit { num: i64, den: i64 } => Timeline, serde_json::json!({"num":30,"den":1});
    SetViewState, "project.set_view_state", "Set timeline view", SessionControl { zoom: f64, scroll: f64, playhead: Tick } => Timeline, serde_json::json!({"zoom":2.0,"scroll":10.0,"playhead":0}), grant = Agent;
    ThumbnailProject, "project.thumbnail", "Refresh project thumbnail", SessionControl {} => Library, serde_json::json!({}), grant = Human, lifecycle = Thumbnail;
    ProviderCatalog, "provider_catalog", "Provider catalog", Read {} => Timeline, serde_json::json!({});
    Redo, "redo", "Redo", SessionControl {} => Redo, serde_json::json!({}), available = CanRedo, grant = Agent, curated = true;
    ReframeApply, "reframe.apply", "Apply a reframe result", Edit { job: super::wire::JobId, replace_manual: bool } => Reframe, serde_json::json!({"job":0,"replace_manual":false}), available = UnlockedItem;
    ReframeClear, "reframe.clear", "Clear reframe crop keys", Edit { item: ItemId } => ReframeShot, serde_json::json!({"item":8}), available = UnlockedItem;
    ReframeGenerate, "reframe.generate", "Analyse subject reframing", JobControl { item: ItemId, target: super::wire::ReframeTarget } => Reframe, serde_json::json!({"item":8,"target":"9:16"}), grant = Agent;
    ReframeSetManual, "reframe.set_manual", "Mark a reframe shot manual", Edit { item: ItemId, at: Tick } => ReframeShot, serde_json::json!({"item":8,"at":0}), available = UnlockedItem;
    Retranslate, "retranslate", "Apply selected cue translations", Edit { item: ItemId, target_language: captions::LanguageTag, texts: std::collections::BTreeMap<NfcText, NfcText>, translation_ref: NfcText, force: bool } => Timeline, serde_json::json!({"item":4,"target_language":"vi","texts":{"cue-1":"Xin chào"},"translation_ref":"example","force":true}), available = UnlockedItem;
    RippleDelete, "ripple_delete", "Ripple delete", Edit { item: ItemId } => Timeline, serde_json::json!({"item":1}), available = UnlockedItem;
    CreateScene, "scene.create", "Create scene", Edit { name: NfcText } => Timeline, serde_json::json!({"name":"Scene 2"});
    DeleteScenes, "scene.delete", "Delete scenes", Edit { scenes: Vec<project::SceneId> } => Timeline, serde_json::json!({"scenes":[]});
    RenameScene, "scene.rename", "Rename scene", Edit { scene: project::SceneId, name: NfcText } => Timeline, serde_json::json!({"scene":0,"name":"Renamed scene"});
    SwitchScene, "scene.switch", "Switch scene", SessionControl { scene: project::SceneId } => Timeline, serde_json::json!({"scene":0}), grant = Agent;
    SetItemParam, "set_item_param", "Set item parameters", Edit { item: ItemId, content: ItemParams } => Timeline, serde_json::json!({"item":1,"content":{"kind":"overlay","params":{"color":[255,255,0,255]}}}), available = UnlockedItem;
    AddLibrarySound, "sounds.add_to_timeline", "Add library sound", Edit { #[serde(default)] placement: Placement, start: Tick, asset: SourceAsset, sound: project::LibrarySound } => Timeline, serde_json::json!({"placement":{"mode":"auto"},"start":0,"asset":{"ephemeral":false,"id":99,"path":"$output/sound.wav","content_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"probe":{"format":"wav","duration":254016000_i64,"streams":[{"index":0,"kind":"Audio","codec":"pcm_s16le","time_base":[1,8000],"duration":254016000_i64,"start":0,"frame_rate":null,"frame_count":null,"variable_frame_rate":null,"video":null,"audio":{"sample_rate":8000,"channels":1,"initial_padding":0,"trailing_padding":0}}]}},"sound":{"id":1,"name":"Whoosh","author":"Freesound user","licence":"Creative Commons 0","attribution":"Whoosh by Freesound user (Creative Commons 0)","source_url":"https://freesound.org/s/1/","audio_url":"https://cdn.freesound.org/previews/1/1-hq.mp3","downloads":10,"duration_ms":1000}}), grant = Human;
    SoundsSave, "sounds.save", "Save library sound", Read { sound: super::wire::SoundEntry } => Timeline, serde_json::json!({"sound":{"id":1,"name":"Whoosh","author":"Freesound user","licence":"Creative Commons 0","attribution":"Whoosh by Freesound user (Creative Commons 0)","source_url":"https://freesound.org/s/1/","audio_url":"https://cdn.freesound.org/previews/1/1-hq.mp3","duration_ms":1000,"downloads":10}}), grant = Human;
    SoundsSearch, "sounds.search", "Search sound library", Read { query: String, page: u32, page_size: u32, commercial_only: bool } => Timeline, serde_json::json!({"query":"whoosh","page":1,"page_size":15,"commercial_only":true}), grant = Human;
    SoundsUnsave, "sounds.unsave", "Unsave library sound", Read { id: u64 } => Timeline, serde_json::json!({"id":1}), grant = Human;
    SplitAt, "split_at", "Split item", Edit { item: ItemId, at: Tick } => Timeline, serde_json::json!({"item":1,"at":8467200000_i64}), available = UnlockedItem;
    StickersInsert, "stickers.insert", "Insert sticker", Edit { #[serde(default)] placement: Placement, sticker: String, start: Tick, duration: Tick } => Timeline, serde_json::json!({"placement":{"mode":"auto"},"sticker":"flags:vn","start":0,"duration":127008000000_i64});
    StickersSearch, "stickers.search", "Search stickers", Read { #[serde(default)] query: String, #[serde(default)] category: super::wire::StickerCategory, #[serde(default)] limit: Option<u32> } => Timeline, serde_json::json!({"query":"asia","category":"flags","limit":20});
    ApplyTextAnimation, "text.apply_animation", "Apply text animation", Edit { item: ItemId, preset: TextAnimationPreset } => Text, serde_json::json!({"item":3,"preset":"fade_in_out"}), available = UnlockedItem, grant = Agent;
    ApplyTextPreset, "text.apply_preset", "Apply saved text preset", Edit { items: Vec<ItemId>, preset: project::TextPresetId } => TextPresetBank, serde_json::json!({"items":[3],"preset":100}), available = UnlockedItem, grant = Agent;
    SaveTextPreset, "text.save_preset", "Save text preset", Edit { item: ItemId, name: NfcText } => TextPresetBank, serde_json::json!({"item":3,"name":"Style 1"}), available = UnlockedItem, grant = Agent;
    AddTrack, "timeline.add_track", "Add track", Edit { name: NfcText, kind: TrackKind, #[serde(default)] placement: TrackPlacement } => Timeline, serde_json::json!({"name":"Added","kind":"video"});
    AlignItems, "timeline.align_items", "Align items", Edit { items: Vec<ItemId>, alignment: project::ItemAlignment, at: Tick } => Timeline, serde_json::json!({"items":[1],"alignment":"left","at":0}), available = UnlockedItem;
    TimelineCommitPreview, "timeline.commit_preview", "Commit timeline preview", SessionControl {} => Gesture, serde_json::json!({}), available = GestureActive, grant = Human;
    TimelineDiscardPreview, "timeline.discard_preview", "Discard timeline preview", SessionControl {} => Gesture, serde_json::json!({}), available = GestureActive, grant = Human;
    DistributeItems, "timeline.distribute_items", "Distribute items evenly", Edit { items: Vec<ItemId>, axis: project::DistributionAxis, at: Tick } => Arrange, serde_json::json!({"items":[1,3,5],"axis":"horizontal","at":0}), available = UnlockedItem;
    DuplicateItems, "timeline.duplicate", "Duplicate items on new tracks", Edit { items: Vec<ItemId> } => Timeline, serde_json::json!({"items":[5]}), available = UnlockedItem;
    TimelineEdit, "timeline.edit", "Edit timeline with ripple policy", Edit { commands: Vec<Command>, ripple: bool } => Timeline, serde_json::json!({"commands":[{"command":"delete","params":{"items":[1]}}],"ripple":true});
    ExtractAudio, "timeline.extract_audio", "Extract source audio", Edit { items: Vec<ItemId> } => SourceAudio, serde_json::json!({"items":[5]}), available = VideoItem;
    FillerCandidates, "timeline.filler_candidates", "List filler candidates", Read { language: captions::LanguageTag, include_markers: bool } => Words, serde_json::json!({"language":"en","include_markers":false}), available = CaptionItem;
    InsertItem, "timeline.insert", "Insert item", Edit { #[serde(default)] placement: Placement, item: NewItem } => Timeline, serde_json::json!({"placement":{"mode":"auto"},"item":{"animations":{},"source_asset":null,"source_range":{"start":0,"end":8467200000_i64},"start":0,"content":{"kind":"overlay","params":{"color":[255,0,0,255]}}}});
    TimelineMoveItems, "timeline.move_items", "Move timeline group", Edit { items: Vec<ItemId>, delta: Tick, #[serde(default)] placement: Option<Placement> } => Timeline, serde_json::json!({"items":[1],"delta":0,"placement":null}), available = UnlockedItem;
    PauseCandidates, "timeline.pause_candidates", "List pause candidates", JobControl { min_pause_ms: i64, pad_ms: i64, crossfade_ms: i64 } => Timeline, serde_json::json!({"min_pause_ms":500,"pad_ms":20,"crossfade_ms":20}), grant = Agent;
    TimelinePreview, "timeline.preview", "Preview timeline edits", SessionControl { commands: Vec<Command> } => Timeline, serde_json::json!({"commands":[{"command":"timeline.move_items","params":{"items":[1],"delta":0}}]}), grant = Human;
    RecoverAudio, "timeline.recover_audio", "Recover source audio", Edit { items: Vec<ItemId> } => Timeline, serde_json::json!({"items":[5]}), available = VideoItem;
    RemoveFillers, "timeline.remove_fillers", "Remove reviewed fillers", Edit { ids: Vec<NfcText>, confirm_edited: bool } => Words, serde_json::json!({"ids":["filler:4:cue-1:0"],"confirm_edited":false}), available = CaptionItem;
    RemovePauses, "timeline.remove_pauses", "Remove reviewed pauses", Edit { spans: Vec<OutputSpan>, crossfade_ms: i64 } => Timeline, serde_json::json!({"spans":[{"start":63504000000_i64,"end":127008000000_i64}],"crossfade_ms":20});
    SetItemsFlipped, "timeline.set_flipped", "Flip items", Edit { items: Vec<ItemId>, axis: project::FlipAxis, flipped: bool } => Timeline, serde_json::json!({"items":[1],"axis":"horizontal","flipped":true}), available = UnlockedItem;
    SetItemsHidden, "timeline.set_hidden", "Hide items", Edit { items: Vec<ItemId>, hidden: bool } => Timeline, serde_json::json!({"items":[1],"hidden":true}), available = UnlockedItem;
    SetItemField, "timeline.set_item_param", "Set item parameter", Edit { item: ItemId, patch: ItemParamPatch } => Timeline, serde_json::json!({"item":1,"patch":{"field":"blendMode","value":"multiply"}}), available = UnlockedItem;
    SetItemParamMany, "timeline.set_item_param_many", "Set shared item field", Edit { items: Vec<ItemId>, patch: ItemParamPatch } => Timeline, serde_json::json!({"items":[1],"patch":{"field":"text_size","value":48.0}});
    SetItemsMuted, "timeline.set_muted", "Mute items", Edit { items: Vec<ItemId>, muted: bool } => Timeline, serde_json::json!({"items":[5],"muted":true}), available = UnlockedItem;
    TimelineSetRipple, "timeline.set_ripple", "Set ripple editing", SessionControl { enabled: bool } => Timeline, serde_json::json!({"enabled":false}), available = NoTransaction, grant = Agent;
    SetSpeed, "timeline.set_speed", "Set clip speed", Edit { items: Vec<ItemId>, speed: time::Speed, maintain_pitch: bool } => Timeline, serde_json::json!({"items":[5],"speed":2.0,"maintain_pitch":true}), available = RetimableItem;
    TimelineSplitLeft, "timeline.split_left", "Split items and remove the left part", Edit { items: Vec<ItemId>, at: Tick } => Timeline, serde_json::json!({"items":[5],"at":381024000000_i64}), available = UnlockedItem;
    TimelineSplitRight, "timeline.split_right", "Split items and remove the right part", Edit { items: Vec<ItemId>, at: Tick } => Timeline, serde_json::json!({"items":[5],"at":381024000000_i64}), available = UnlockedItem;
    TimelineTrim, "timeline.trim", "Trim timeline group", Edit { items: Vec<ItemId>, edge: TrimEdge, delta: Tick } => Timeline, serde_json::json!({"items":[1],"edge":"end","delta":-1}), available = UnlockedItem;
    RemoveTrack, "track.remove", "Remove track", Edit { track: TrackId } => Timeline, serde_json::json!({"track":3}), available = RemovableTrack;
    RenameTrack, "track.rename", "Rename track", Edit { track: TrackId, name: NfcText } => Timeline, serde_json::json!({"track":3,"name":"Renamed"});
    ReorderTrack, "track.reorder", "Reorder track", Edit { track: TrackId, index: u32 } => Timeline, serde_json::json!({"track":3,"index":0});
    SetTrackFlags, "track.set_flags", "Set track flags", Edit { track: TrackId, muted: bool, hidden: bool, locked: bool } => Timeline, serde_json::json!({"track":3,"muted":true,"hidden":false,"locked":false});
    SetTrackHidden, "track.set_hidden", "Hide track", Edit { track: TrackId, hidden: bool } => Timeline, serde_json::json!({"track":3,"hidden":true});
    SetTrackLocked, "track.set_locked", "Lock track", Edit { track: TrackId, locked: bool } => Timeline, serde_json::json!({"track":3,"locked":true});
    SetTrackMagnet, "track.set_magnet", "Main track magnet", Edit { enabled: bool } => Timeline, serde_json::json!({"enabled":false});
    SetTrackMuted, "track.set_muted", "Mute track", Edit { track: TrackId, muted: bool } => Timeline, serde_json::json!({"track":3,"muted":true});
    TranscriptDeleteRange, "transcript.delete_range", "Delete transcript words", Edit { word_ids: Vec<WordId> } => Words, serde_json::json!({"word_ids":[{"item":4,"cue":"cue-1","word":0}]}), available = CaptionItem;
    TranscriptMergeCues, "transcript.merge_cues", "Merge transcript cues", Edit { cue_ids: Vec<CueId> } => Words, serde_json::json!({"cue_ids":[{"item":4,"cue":"cue-1"},{"item":4,"cue":"cue-2"}]}), available = CaptionItem;
    AddTransition, "transition.add", "Add video transition", Edit { track: TrackId, from: ItemId, to: ItemId, kind: String, duration: Tick, alignment: project::TransitionAlignment } => TransitionCut, serde_json::json!({"track":1,"from":10,"to":11,"kind":"cross_dissolve","duration":20,"alignment":"center"});
    MoveTransition, "transition.move", "Move video transition", Edit { transition: project::TransitionId, start: Tick } => Transition, serde_json::json!({"transition":13,"start":40});
    RemoveTransition, "transition.remove", "Remove video transition", Edit { transition: project::TransitionId } => Transition, serde_json::json!({"transition":13});
    SetTransitionDuration, "transition.set_duration", "Resize video transition", Edit { transition: project::TransitionId, duration: Tick } => Transition, serde_json::json!({"transition":13,"duration":20});
    SetTransitionParam, "transition.set_param", "Set video transition parameter", Edit { transition: project::TransitionId, param: String, value: project::TransitionValue } => Transition, serde_json::json!({"transition":13,"param":"hold","value":{"type":"float","value":40.0}});
    TrimEnd, "trim_end", "Trim end", Edit { item: ItemId, at: Tick } => Timeline, serde_json::json!({"item":1,"at":8467200000_i64}), available = UnlockedItem;
    TrimStart, "trim_start", "Trim start", Edit { item: ItemId, at: Tick } => Timeline, serde_json::json!({"item":1,"at":8467200000_i64}), available = UnlockedItem;
    Undo, "undo", "Undo", SessionControl {} => History, serde_json::json!({}), available = CanUndo, grant = Agent, curated = true;
}

/// First declared id that breaks ascending byte order, with the id before it.
/// Catalog generation refuses before writing because the registry preserves
/// declaration order, so this one scan gates both source and generated order.
pub fn declaration_order_error<'a>(ids: &[&'a str]) -> Option<(&'a str, &'a str)> {
    ids.iter()
        .copied()
        .zip(ids.iter().copied().skip(1))
        .find_map(|(before, after)| (before > after).then_some((before, after)))
}

pub fn registry() -> &'static [CommandSpec] {
    static REGISTRY: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut schema = schemars::schema_for!(Command).to_value();
        super::wire::close_schema(&mut schema);
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
            .into_root_schema_for::<super::wire::ControlResult>()
            .to_value();
        super::wire::close_schema(&mut output);
        declarations()
            .into_iter()
            .map(|(id, label, effect, example, facts)| {
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
                    limits: super::wire::LIMITS,
                    params_schema: params,
                    output_schema: output.clone(),
                    example,
                    availability: facts.available,
                    grant: facts.grant,
                    curated: facts.curated,
                    lifecycle: facts.lifecycle,
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
    CaptionPreset(project::CaptionPresetId),
    TextPreset(project::TextPresetId),
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
