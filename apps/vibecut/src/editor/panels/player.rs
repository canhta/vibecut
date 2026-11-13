//! Owns preview presentation state behind the player panel interface.
pub mod crop;
mod mask_path;
mod overlay;
mod snap;
mod state;
mod text;
mod transform;
mod view;
mod workflow;

use crate::{editor::Context, egui};
pub use overlay::{Guide, OverlayVisibility};
use project::ItemId;
pub use snap::PREVIEW_SNAP_PIXELS;
pub(crate) use state::{KeyPick, State};
pub use state::{PreviewAxis, PreviewGuide, TextCaret};
pub(in crate::editor) use workflow::format_timecode;

/// Requests are applied by the frame after rendering releases its read borrow.
pub(crate) enum Action {
    Command(String),
    Retry,
    CommitTimecode,
    Zoom(f32),
    Select(ItemId),
    ClearSelection,
    /// Absolute transform values staged from one gesture origin.
    TransformPreview {
        item: ItemId,
        values: Vec<(project::TransformField, f64)>,
    },
    /// Close the staged gesture, publishing one undo step when `commit`.
    TransformEnd {
        commit: bool,
    },
    /// Open one inline text session and select its item.
    TextBegin {
        item: ItemId,
    },
    /// Stage the whole text buffer as the session's absolute candidate.
    TextPreview {
        item: ItemId,
        content: String,
    },
    /// Close the inline text session; `commit` publishes one undo step.
    TextEnd {
        commit: bool,
    },
    /// Select one mask point, toggling it when `toggle`.
    MaskPointSelect {
        target: crate::editor::selection::MaskPointSelection,
        toggle: bool,
    },
    /// Leave the mask-point scope without changing the item selection.
    MaskPointClear,
    /// Select the anchors a marquee covered, adding to the scope when `toggle`.
    MaskPointMarquee {
        item: ItemId,
        mask: String,
        points: Vec<usize>,
        toggle: bool,
    },
    /// Stage one absolute mask path candidate from a handle drag.
    MaskPathPreview {
        item: ItemId,
        mask: String,
        path: project::MaskPath,
    },
    /// Close a mask path gesture, publishing one undo step when `commit`.
    MaskPathEnd {
        commit: bool,
    },
    /// Insert one point on the nearest cubic segment.
    MaskPointAdd {
        item: ItemId,
        mask: String,
        segment: usize,
        t: f64,
    },
    /// A pixel of the rendered preview frame for the armed chroma-key eyedropper.
    PickKeyColor(project::Color),
    /// One preview preference edit, recorded in the app's preference history.
    SetPreference {
        key: &'static str,
        value: serde_json::Value,
    },
    Remember {
        id: String,
        icon: Option<&'static str>,
        tooltip: Option<String>,
        response: egui::Response,
    },
}

pub(crate) fn show(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui) -> Vec<Action> {
    view::show(state, cx, ui)
}
