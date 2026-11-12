//! Keyframe graph popover: the keys it edits, its working curve and the handle maths.
//!
//! The popover never writes the project. Every change is staged as a preview
//! command inside one engine gesture; Apply publishes that gesture as one undo
//! step and Cancel discards it, so the pre-open state is restored exactly.

use super::{KeyframeSelection, egui};
use project::{ColorComponent, EasingCurve};

/// Side of the square plot, in points.
pub(in crate::editor::timeline) const PLOT: f32 = 200.0;
/// Room around the plot so a handle at the edge stays grabbable.
pub(in crate::editor::timeline) const MARGIN: f32 = 14.0;
/// Pointer distance within which a handle is picked.
const GRAB: f32 = 14.0;
/// Visible value range: y = 0 is the start value and y = 1 the end value.
const Y_MIN: f64 = -0.5;
const Y_MAX: f64 = 1.5;

/// Which control point of the curve a drag moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor::timeline) enum Handle {
    /// `(x1, y1)`, the outgoing handle of the left key.
    Out,
    /// `(x2, y2)`, the incoming handle of the right key.
    In,
}

/// One open graph popover.
#[derive(Clone, Debug)]
pub(in crate::editor::timeline) struct GraphPopover {
    pub(in crate::editor::timeline) anchor: egui::Pos2,
    /// The keys whose outgoing segments it edits, fixed when it opened.
    pub(in crate::editor::timeline) keys: Vec<KeyframeSelection>,
    /// Active component tab, an index into [`components`].
    pub(in crate::editor::timeline) tab: usize,
    pub(in crate::editor::timeline) grab: Option<Handle>,
    /// A gesture is open and holds staged previews.
    pub(in crate::editor::timeline) staged: bool,
}

/// The easable segments among `keys` with each one's current normalized curve.
pub(in crate::editor::timeline) fn segments(
    project: &project::Project,
    keys: &[KeyframeSelection],
) -> Vec<(KeyframeSelection, EasingCurve)> {
    keys.iter()
        .filter_map(|key| {
            let item = project
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| track.items.iter())
                .find(|item| item.id == key.item)?;
            let curve = item
                .animations
                .get(&key.path)?
                .channel(key.component)
                .ok()?
                .segment_curve(key.time)?;
            Some((key.clone(), curve))
        })
        .collect()
}

/// Distinct colour components among `segments`, in selection order. One entry
/// (including `None`) means the popover shows no tabs.
pub(in crate::editor::timeline) fn components(
    segments: &[(KeyframeSelection, EasingCurve)],
) -> Vec<Option<ColorComponent>> {
    let mut found = Vec::new();
    for (key, _) in segments {
        if !found.contains(&key.component) {
            found.push(key.component);
        }
    }
    found
}

/// Command addresses for `keys`.
pub(in crate::editor::timeline) fn refs<'a>(
    keys: impl IntoIterator<Item = &'a KeyframeSelection>,
) -> Vec<engine::KeyframeRef> {
    keys.into_iter()
        .map(|key| engine::KeyframeRef {
            item: key.item,
            path: key.path.clone(),
            component: key.component,
            time: key.time,
        })
        .collect()
}

/// Screen position of curve point `(x, y)` inside `plot`.
pub(in crate::editor::timeline) fn to_point(plot: egui::Rect, x: f64, y: f64) -> egui::Pos2 {
    let along = ((y - Y_MIN) / (Y_MAX - Y_MIN)) as f32;
    egui::pos2(
        plot.left() + x as f32 * plot.width(),
        plot.bottom() - along * plot.height(),
    )
}

/// Curve point under a screen position, clamped to `x ∈ 0..=1` and the visible y range.
pub(in crate::editor::timeline) fn from_point(plot: egui::Rect, point: egui::Pos2) -> (f64, f64) {
    let x = f64::from((point.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    let along = f64::from((plot.bottom() - point.y) / plot.height());
    (x, (Y_MIN + along * (Y_MAX - Y_MIN)).clamp(Y_MIN, Y_MAX))
}

/// The handle within grabbing distance of `point`, preferring the nearer one.
pub(in crate::editor::timeline) fn nearest(
    plot: egui::Rect,
    curve: EasingCurve,
    point: egui::Pos2,
) -> Option<Handle> {
    let [x1, y1, x2, y2] = curve.0;
    let out = to_point(plot, x1, y1).distance(point);
    let incoming = to_point(plot, x2, y2).distance(point);
    let (handle, distance) = if out <= incoming {
        (Handle::Out, out)
    } else {
        (Handle::In, incoming)
    };
    (distance <= GRAB).then_some(handle)
}

/// `curve` with `handle` moved to the pointer.
pub(in crate::editor::timeline) fn dragged(
    plot: egui::Rect,
    curve: EasingCurve,
    handle: Handle,
    point: egui::Pos2,
) -> EasingCurve {
    let (x, y) = from_point(plot, point);
    let [x1, y1, x2, y2] = curve.0;
    EasingCurve(match handle {
        Handle::Out => [x, y, x2, y2],
        Handle::In => [x1, y1, x, y],
    })
}

/// The cubic as plot points, for painting.
pub(in crate::editor::timeline) fn outline(
    plot: egui::Rect,
    curve: EasingCurve,
) -> Vec<egui::Pos2> {
    let [x1, y1, x2, y2] = curve.0;
    let cubic = |s: f64, a: f64, b: f64| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    (0..=48)
        .map(|step| {
            let s = f64::from(step) / 48.0;
            to_point(plot, cubic(s, x1, x2), cubic(s, y1, y2))
        })
        .collect()
}
