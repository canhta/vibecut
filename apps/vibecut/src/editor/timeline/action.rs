//! Names the requests the timeline panel hands back to the frame.

use crate::editor::timeline::{
    BTreeSet, ItemId, KeyframeSelection, Tick, TrackId, TrackKind, egui,
};

/// Where a dragged clip lands when it is released over another track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum DropTarget {
    /// An existing track.
    Track(TrackId),
    /// A new track of the clip's kind, allocated beside `placement`.
    New {
        kind: TrackKind,
        placement: engine::TrackPlacement,
    },
}

/// A cross-area request. The panel never mutates the session, the
/// selection or the transport itself; the frame applies these in order.
#[derive(Clone, Debug)]
pub(in crate::editor) enum Action {
    /// Run a registered action by id (toolbar, scene sheet, media reveal).
    Activate(String),
    /// Run an engine command on the current selection; the frame prunes the
    /// selection and restarts the preview when the project changed.
    Command {
        id: &'static str,
        params: serde_json::Value,
    },
    /// Rename a scene; the frame reports a refusal back through
    /// `State::keep_scene_rename`.
    RenameScene {
        scene: project::SceneId,
        name: String,
    },
    /// Move the playhead.
    Seek(Tick),
    /// Click selection with the pointer's modifiers.
    Select {
        item: ItemId,
        modifiers: egui::Modifiers,
    },
    /// Live marquee result; the selection is not settled until `SettleSelection`.
    Marquee(BTreeSet<ItemId>),
    /// The marquee ended: announce the selection to the other areas.
    SettleSelection,
    /// Click a keyframe diamond with the pointer's modifiers.
    SelectKeyframe {
        keyframe: KeyframeSelection,
        modifiers: egui::Modifiers,
    },
    /// Live keyframe-lane marquee; settles on release.
    MarqueeKeyframes(BTreeSet<KeyframeSelection>),
    /// The keyframe marquee ended: keep the live result.
    SettleKeyframes,
    /// Click empty lane space: clear the keyframe scope only.
    ClearKeyframes,
    /// Expand or collapse an item's keyframe lanes.
    SetLaneOpen { item: ItemId, open: bool },
    /// Delete the selected keyframes in one undo step.
    DeleteKeyframes(BTreeSet<KeyframeSelection>),
    /// Deselect everything.
    ClearSelection,
    /// Open one preview gesture. `select` is an item to select alone first
    /// because the gesture grabbed an unselected clip.
    BeginGesture { select: Option<ItemId> },
    /// Apply one preview command inside the open gesture.
    Preview(Box<engine::Command>),
    /// Move a clip to another track (allocating one if needed) inside the gesture.
    MoveToTrack {
        item: ItemId,
        start: Tick,
        target: DropTarget,
    },
    /// Publish the open gesture as one undo step.
    EndGesture,
    /// Abandon every open gesture (a drop the panel could not complete).
    CancelInteraction,
    /// Surface an error to the user.
    Fail(String),
}
