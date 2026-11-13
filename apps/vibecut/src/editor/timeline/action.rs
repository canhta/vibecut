//! Names the requests the timeline panel hands back to the frame.

use crate::editor::context::visuals::VisualRequest;
use crate::editor::timeline::{BTreeSet, ItemId, KeyframeSelection, Tick, egui};

/// One timeline gesture command from the group move/trim contract.
///
/// The panel never applies these itself: the frame stages each one with
/// `timeline.preview` and publishes or discards the whole candidate.
#[derive(Clone, Debug)]
pub(in crate::editor) enum Gesture {
    /// `timeline.move_items`: one delta keeps the selected group rigid.
    Move {
        items: Vec<ItemId>,
        delta: Tick,
        /// `None` retains each member's track; an explicit placement moves the
        /// whole selection and may allocate one destination per source track.
        placement: Option<engine::Placement>,
    },
    /// `timeline.trim`: one delta moves every selected start or end edge.
    Trim {
        items: Vec<ItemId>,
        edge: engine::TrimEdge,
        delta: Tick,
    },
}

impl Gesture {
    /// The typed command this gesture stages.
    pub(in crate::editor) fn command(&self) -> engine::Command {
        match self {
            Self::Move {
                items,
                delta,
                placement,
            } => engine::Command::TimelineMoveItems {
                items: items.clone(),
                delta: *delta,
                placement: *placement,
            },
            Self::Trim { items, edge, delta } => engine::Command::TimelineTrim {
                items: items.clone(),
                edge: *edge,
                delta: *delta,
            },
        }
    }
    /// The command would leave every member where the gesture found it.
    pub(in crate::editor) fn is_noop(&self) -> bool {
        match self {
            Self::Move {
                delta, placement, ..
            } => *delta == Tick::ZERO && placement.is_none(),
            Self::Trim { delta, .. } => *delta == Tick::ZERO,
        }
    }
}

/// A cross-area request. The panel never mutates the session, the
/// selection or the transport itself; the frame applies these in order.
#[derive(Clone, Debug)]
pub(in crate::editor) enum Action {
    /// Run a registered action by id (toolbar, scene sheet, media reveal).
    Activate(String),
    /// Run one typed engine command on the current selection; the frame prunes
    /// the selection and restarts the preview when the project changed.
    Command(Box<engine::Command>),
    /// Open the native media picker to replace one clip's source; the window owns the dialog.
    ReplaceMedia(ItemId),
    /// Submit one registered media generation job for a pending clip visual.
    Generate(VisualRequest),
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
    /// Select an applied transition on the timeline.
    SelectTransition(project::TransitionId),
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
    /// Open one explicit gesture. `select` is an item to select alone first
    /// because the gesture grabbed an unselected clip.
    BeginGesture { select: Option<ItemId> },
    /// Apply one preview command inside the open gesture.
    Preview(Box<engine::Command>),
    /// Publish the open gesture as one undo step.
    EndGesture,
    /// Discard the open gesture and show the project it started from.
    DiscardGesture,
    /// Persist the graph popover's tab.
    SetEasingTab(crate::preferences::EasingTab),
    /// Save a normalized curve as the next user easing preset.
    SaveEasing([f64; 4]),
    /// Arm the group-preview contract: the next `PreviewGesture` starts it.
    BeginPreview { select: Option<ItemId> },
    /// Replace the whole staged candidate with this group command.
    PreviewGesture(Box<Gesture>),
    /// Publish the staged candidate as one undo step.
    EndPreview,
    /// Abandon every open gesture (a drop the panel could not complete).
    CancelInteraction,
    /// Surface an error to the user.
    Fail(String),
}
