//! Applies the actions the timeline panel returns, through the session port.
use super::super::{Action, Gesture, State};
use crate::editor::{Command, Port, Request, Tick};

/// Apply the actions in order. The selection scopes, the open scrub and the gesture
/// openers belong to the frame, so the run stops at the first action that needs them
/// and hands it back with everything after it; the frame applies that action and calls
/// again with the rest.
pub(in crate::editor) fn apply(
    state: &mut State,
    port: &mut Port<'_>,
    actions: Vec<Action>,
) -> Vec<Action> {
    let mut actions = actions.into_iter();
    while let Some(action) = actions.next() {
        match action {
            Action::Activate(id) => port.request(Request::Action(id)),
            Action::Command(command) => command_on_selection(port, *command),
            Action::ReplaceMedia(item) => port.request(Request::ReplaceMedia(item)),
            Action::Generate(request) => {
                if let Err(error) = state.generate(&mut *port.session, request, port.read.budget) {
                    port.fail(error.to_string());
                }
            }
            Action::RenameScene { scene, name } => {
                let command = Command::RenameScene {
                    scene,
                    name: name.clone().into(),
                };
                if port.apply_result(command).is_err() {
                    state.keep_scene_rename(scene, name);
                }
            }
            Action::Seek(at) => {
                let at = at.clamp(Tick::ZERO, port.session.project().duration());
                port.request(Request::Seek(at));
            }
            Action::SettleKeyframes => {}
            Action::SetLaneOpen { item, open } => set_lane_open(state, port, item, open),
            Action::Preview(command) => preview_gesture(port, *command),
            Action::EndGesture => end_drag_gesture(port),
            Action::DiscardGesture => discard_gesture(port),
            Action::SetEasingTab(tab) => {
                let mut easing = port.read.preferences.easing.clone();
                easing.tab = tab;
                port.set_preference("easing", serde_json::json!(easing));
            }
            Action::SaveEasing(curve) => save_easing(port, curve),
            Action::PreviewGesture(gesture) => preview_timeline(state, port, *gesture),
            Action::EndPreview => end_timeline_preview(state, port),
            Action::Fail(message) => {
                state.refuse(message.clone());
                port.fail(message);
            }
            frame @ (Action::Select { .. }
            | Action::SelectTransition(_)
            | Action::Marquee(_)
            | Action::SettleSelection
            | Action::SelectKeyframe { .. }
            | Action::MarqueeKeyframes(_)
            | Action::ClearKeyframes
            | Action::DeleteKeyframes(_)
            | Action::ClearSelection
            | Action::BeginGesture { .. }
            | Action::BeginPreview { .. }
            | Action::CancelInteraction) => {
                let mut rest = vec![frame];
                rest.extend(actions);
                return rest;
            }
        }
    }
    Vec::new()
}

/// Run a typed engine command on the current selection (track edits).
pub(in crate::editor) fn command_on_selection(port: &mut Port<'_>, command: Command) {
    let before = port.session.project().clone();
    match port.session.apply(command) {
        Ok(_) => {
            port.request(Request::PruneSelection);
            if !std::sync::Arc::ptr_eq(&before, port.session.project()) {
                port.sync();
            }
        }
        Err(error) => port.fail(error.to_string()),
    }
}

/// Persist one item's expanded lane choice for the owning project only.
fn set_lane_open(state: &mut State, port: &mut Port<'_>, item: project::ItemId, open: bool) {
    state.set_lane(item, open);
    let project = port.session.project().data().id;
    let mut lanes = port.read.preferences.timeline_lanes.clone();
    let items: std::collections::BTreeSet<u64> =
        state.expanded_lanes().into_iter().map(|id| id.0).collect();
    if items.is_empty() {
        lanes.remove(&project);
    } else {
        lanes.insert(project, items);
    }
    port.set_preference("timeline_lanes", serde_json::json!(lanes));
}

/// Append `curve` to the saved easing presets under the next free name.
fn save_easing(port: &mut Port<'_>, curve: [f64; 4]) {
    let mut easing = port.read.preferences.easing.clone();
    if easing.saved.len() >= crate::preferences::MAX_SAVED_EASINGS
        || easing.saved.iter().any(|saved| saved.curve == curve)
    {
        return;
    }
    let name = easing.next_name();
    easing
        .saved
        .push(crate::preferences::SavedEasing { name, curve });
    port.set_preference("easing", serde_json::json!(easing));
}

fn preview_gesture(port: &mut Port<'_>, command: Command) {
    // A refused or cancelled gesture must not turn its previews into edits.
    if !port.session.gesture_active() {
        return;
    }
    // Absolute pointer targets are rebuilt from the shared preview baseline,
    // so reversing a drag also reverses every ripple shift.
    let command = match command {
        command @ (Command::MoveItems { .. }
        | Command::TrimStart { .. }
        | Command::TrimEnd { .. }) => Command::TimelinePreview {
            commands: vec![command],
        },
        command => command,
    };
    let previous = port
        .session
        .preview_project()
        .unwrap_or(port.session.project())
        .clone();
    match port.session.apply(command) {
        Ok(_) => {
            let current = port
                .session
                .preview_project()
                .unwrap_or(port.session.project());
            if previous.as_ref() != current.as_ref() {
                port.sync();
            }
        }
        Err(error) => port.fail(error.to_string()),
    }
}

/// Stage one `timeline.move_items`/`timeline.trim`; each call replaces the
/// whole candidate from the gesture entry snapshot.
fn preview_timeline(state: &mut State, port: &mut Port<'_>, gesture: Gesture) {
    if !state.preview_armed() {
        return;
    }
    let command = gesture.command();
    let staged = Command::TimelinePreview {
        commands: vec![command.clone()],
    };
    let previous = port
        .session
        .preview_project()
        .unwrap_or(port.session.project())
        .clone();
    match port.session.apply(staged) {
        Ok(_) => {
            // `project()` stays committed; the player follows the staged
            // candidate (including a reversal back to the entry state) so a
            // trim shows its frame live.
            state.mark_preview(command.kind(), gesture.is_noop());
            let current = port
                .session
                .preview_project()
                .unwrap_or(port.session.project());
            if previous.as_ref() != current.as_ref() {
                port.sync();
            }
        }
        Err(error) => {
            // The engine refused the whole candidate: drop the gesture and
            // show its reason instead of leaving a stale preview.
            state.abandon_gesture();
            state.refuse(error.to_string());
            discard_staged_preview(state, port);
        }
    }
}

/// Publish the staged candidate as one undo step.
fn end_timeline_preview(state: &mut State, port: &mut Port<'_>) {
    if state.preview_command().is_some() {
        // A reversal can leave every member at its entry position while
        // item order shifted; discard it so no undo step is published.
        if state.preview_noop() {
            discard_staged_preview(state, port);
        } else {
            match port.session.apply(Command::TimelineCommitPreview {}) {
                Ok(outcome) if outcome.changed => port.sync(),
                Ok(_) => {}
                Err(error) => {
                    // A refused publication must not leave the gesture open.
                    port.fail(error.to_string());
                    let _ = port.session.apply(Command::TimelineDiscardPreview {});
                }
            }
            state.clear_preview();
        }
    } else {
        state.clear_preview();
    }
    port.request(Request::PruneSelection);
}

/// Restore the entry project without a history entry.
fn discard_staged_preview(state: &mut State, port: &mut Port<'_>) {
    if port.session.gesture_active()
        && let Err(error) = port.session.apply(Command::TimelineDiscardPreview {})
    {
        port.fail(error.to_string());
    }
    state.clear_preview();
}

/// Escape on a timeline drag: discard the staged candidate.
pub(in crate::editor) fn discard_timeline_preview(state: &mut State, port: &mut Port<'_>) {
    if state.preview_command().is_some() {
        discard_staged_preview(state, port);
    } else {
        state.clear_preview();
        if port.session.gesture_active()
            && let Err(error) = port.session.cancel_gesture()
        {
            port.fail(error.to_string());
        }
    }
}

/// Publish the open gesture as one undo step.
fn end_drag_gesture(port: &mut Port<'_>) {
    if port.session.gesture_active() {
        match port.session.commit_gesture() {
            Ok(outcome) if outcome.changed => port.sync(),
            Ok(_) => {}
            Err(error) => port.fail(error.to_string()),
        }
    }
    port.request(Request::PruneSelection);
}

/// Discard the open gesture and show the project it started from.
pub(in crate::editor) fn discard_gesture(port: &mut Port<'_>) {
    if port.session.gesture_active() {
        match port.session.cancel_gesture() {
            // Closing a preview lifetime fences even a net-zero drag.
            Ok(_) => port.sync(),
            Err(error) => port.fail(error.to_string()),
        }
    }
}

/// A selection write ends the graph popover's open edit: close it and drop its staged
/// previews.
pub(in crate::editor) fn release_graph(state: &mut State, port: &mut Port<'_>) {
    if state.graph_popover_open() {
        state.escape_graph();
        discard_gesture(port);
    }
}
