use captions::NfcText;
use media::{AssetId, SourceAsset};
use project::{BookmarkId, Color, ItemId, ItemParams, TrackId, TrackKind};
use serde::{Deserialize, Serialize};
use time::{SourceRange, Tick};

/// Required parameter field and its Rust/Serde wire type; not JSON Schema.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct FieldSchema {
    pub name: &'static str,
    pub wire_type: &'static str,
}
/// Registry metadata uses the same declaration as typed commands and JSON dispatch.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub params_schema: &'static [FieldSchema],
}
impl CommandSpec {
    /// Coarse session availability. Target/parameter validation occurs at execution.
    pub fn enabled(&self, session: &crate::Session) -> bool {
        match self.id {
            "begin_gesture" => !session.transaction_active(),
            "commit_gesture" | "cancel_gesture" => session.gesture_active(),
            "undo" => session.can_undo(),
            "redo" => session.can_redo(),
            "set_track_flags" => !session.project().data().timeline.tracks.is_empty(),
            "remove_track" | "insert_item" => session
                .project()
                .data()
                .timeline
                .tracks
                .iter()
                .any(|t| !t.locked),
            "move_item"
            | "move_items"
            | "split_at"
            | "trim_start"
            | "trim_end"
            | "delete"
            | "ripple_delete"
            | "set_item_param"
            | "keyframes.set_param"
            | "keyframes.add"
            | "keyframes.remove"
            | "keyframes.retime"
            | "keyframes.set_handle"
            | "keyframes.scale_time" => session
                .project()
                .data()
                .timeline
                .tracks
                .iter()
                .any(|t| !t.locked && !t.items.is_empty()),
            _ => true,
        }
    }
}
macro_rules! commands {
    ($($variant:ident, $id:literal, $label:literal { $($(#[$attr:meta])* $field:ident : $ty:ty),* $(,)? });* $(;)?) => {
        /// All project mutations. Fields are required; unknown JSON fields are refused.
        #[derive(Clone, Debug, Serialize, Deserialize)]
        #[serde(tag = "command", content = "params", deny_unknown_fields)]
        pub enum Command {
            $(#[serde(rename = $id)] $variant { $($(#[$attr])* $field: $ty),* }),*
        }
        static REGISTRY: &[CommandSpec] = &[
            $(CommandSpec { id: $id, label: $label, params_schema: &[
                $(FieldSchema { name: stringify!($field), wire_type: stringify!($ty) }),*
            ] }),*
        ];
        impl Command {
            pub fn id(&self) -> &'static str {
                match self { $(Self::$variant { .. } => $id),* }
            }
        }
    };
}
// Option allows explicit null, but command fields must still be present.
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
commands! {
    AddAsset, "add_asset", "Add asset" { asset: SourceAsset };
    AddTrack, "add_track", "Add track" { name: NfcText, kind: TrackKind };
    RemoveTrack, "remove_track", "Remove track" { track: TrackId };
    SetTrackFlags, "set_track_flags", "Set track flags" { track: TrackId, muted: bool, hidden: bool, locked: bool };
    InsertItem, "insert_item", "Insert item" { track: TrackId, item: NewItem };
    MoveItem, "move_item", "Move item" { item: ItemId, track: TrackId, start: Tick };
    MoveItems, "move_items", "Move items" { moves: Vec<Move> };
    SplitAt, "split_at", "Split item" { item: ItemId, at: Tick };
    TrimStart, "trim_start", "Trim start" { item: ItemId, at: Tick };
    TrimEnd, "trim_end", "Trim end" { item: ItemId, at: Tick };
    Delete, "delete", "Delete items" { items: Vec<ItemId> };
    RippleDelete, "ripple_delete", "Ripple delete" { item: ItemId };
    SetItemParam, "set_item_param", "Set item parameters" { item: ItemId, content: ItemParams };
    SetAnimatedParam, "keyframes.set_param", "Set animated parameter" { item: ItemId, path: project::AnimationPath, param: project::Param };
    AddKeyframe, "keyframes.add", "Add keyframe" { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] component: Option<project::ColorComponent>, key: project::Keyframe };
    RemoveKeyframe, "keyframes.remove", "Remove keyframe" { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] component: Option<project::ColorComponent>, time: Tick };
    RetimeKeyframe, "keyframes.retime", "Move keyframe" { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] component: Option<project::ColorComponent>, from: Tick, to: Tick };
    SetKeyframeHandle, "keyframes.set_handle", "Set keyframe handle" { item: ItemId, path: project::AnimationPath, #[serde(deserialize_with = "required_option")] component: Option<project::ColorComponent>, time: Tick, side: project::HandleSide, #[serde(deserialize_with = "required_option")] handle: Option<project::CurveHandle>, tangent: project::TangentMode };
    ScaleKeyframeTime, "keyframes.scale_time", "Stretch animation time" { item: ItemId, numerator: i64, denominator: i64 };
    AddBookmark, "add_bookmark", "Add bookmark" { start: Tick, duration: Tick, note: NfcText, color: Color };
    Batch, "batch", "Batch edit" { commands: Vec<Command> };
    BeginGesture, "begin_gesture", "Begin gesture" {};
    CommitGesture, "commit_gesture", "Commit gesture" {};
    CancelGesture, "cancel_gesture", "Cancel gesture" {};
    Undo, "undo", "Undo" {};
    Redo, "redo", "Redo" {};
}
pub fn registry() -> &'static [CommandSpec] {
    REGISTRY
}

/// Item input; the engine allocates its identity only on successful commit.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewItem {
    pub animations: project::Animations,
    pub source_asset: Option<AssetId>,
    pub source_range: SourceRange,
    pub start: Tick,
    pub content: ItemParams,
}
/// Absolute output destination for a simultaneous move.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Move {
    pub item: ItemId,
    pub track: TrackId,
    pub start: Tick,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatedId {
    Track(TrackId),
    Item(ItemId),
    Bookmark(BookmarkId),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub changed: bool,
    pub created: Vec<CreatedId>,
}
