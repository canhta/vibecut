#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
use super::timeline_ui::{fixture, harness, widget};
use egui_kittest::kittest::{By, Queryable};
use time::{TICKS_PER_SECOND, Tick};
use vibecut::{PREFERENCE_COMMAND, actions, egui};

const SECOND: i64 = TICKS_PER_SECOND;

#[test]
fn default_split_key_mutates_history_and_undo_redo_restore_it() {
    let fixture = fixture();
    let id = fixture.video_a;
    let mut editor = fixture.editor;
    editor.selection.insert(id);
    editor
        .playback
        .seek(Tick(3 * SECOND), false, editor.session.project().clone())
        .unwrap();
    let before = editor.session.project().clone();
    let mut harness = harness(editor);
    harness.run_steps(3);

    harness.key_press(egui::Key::S);
    harness.run_steps(3);
    assert_ne!(harness.state().session.project(), &before);
    let after = harness.state().session.project().clone();

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(3);
    assert_eq!(harness.state().session.project(), &before);

    harness.key_press_modifiers(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::Z,
    );
    harness.run_steps(3);
    assert_eq!(harness.state().session.project(), &after);
}

#[test]
fn blocked_contexts_swallow_global_actions() {
    let fixture = fixture();
    let mut editor = fixture.editor;
    assert!(editor.preferences.snapping);

    editor.activate("scene.open").unwrap();
    let mut harness = harness(editor);
    harness.run_steps(3);
    harness.key_press(egui::Key::N);
    harness.run_steps(2);
    assert!(
        harness.state().preferences.snapping,
        "scene sheet must block N"
    );

    harness.state_mut().activate("scene.close").unwrap();
    harness.run_steps(2);
    harness.state_mut().set_recording_active(true);
    harness.key_press(egui::Key::N);
    harness.run_steps(2);
    assert!(
        harness.state().preferences.snapping,
        "recording must block N"
    );
    harness.state_mut().set_recording_active(false);

    let popup = egui::Id::unique("keybindings.popup");
    egui::Popup::open_id(&harness.ctx, popup);
    harness.key_press(egui::Key::N);
    harness.run_steps(1);
    assert!(
        harness.state().preferences.snapping,
        "an open menu must block N"
    );
    egui::Popup::close_id(&harness.ctx, popup);

    harness.state_mut().activate("project.rename").unwrap();
    harness.run_steps(3);
    harness.key_press(egui::Key::N);
    harness.run_steps(2);
    assert!(
        harness.state().preferences.snapping,
        "focused project name must block N"
    );
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    harness.key_press(egui::Key::N);
    harness.run_steps(2);
    assert!(
        !harness.state().preferences.snapping,
        "Escape leaves the input and releases global dispatch"
    );
}

#[test]
fn escape_cancels_the_drag_before_deselecting() {
    let fixture = fixture();
    let id = fixture.video_b;
    let mut editor = fixture.editor;
    editor.selection.insert(id);
    let before = editor.session.project().clone();
    let mut harness = harness(editor);
    harness.run_steps(3);
    let from = widget(&harness, &format!("timeline.item.{}", id.0)).center();
    harness.drag_at(from);
    harness.run_steps(1);
    harness.hover_at(from + egui::vec2(40.0, 0.0));
    harness.run_steps(2);

    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().selection, [id].into_iter().collect());
    assert!(!harness.state().session.gesture_active());
    assert_eq!(harness.state().session.project(), &before);

    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(harness.state().selection.is_empty());
    harness.drop_at(from);
    harness.run_steps(2);
}

#[test]
fn pending_actions_are_typed_unavailable_and_never_mutate_the_project() {
    let fixture = fixture();
    let mut editor = fixture.editor;
    let project = editor.session.project().clone();
    for (action, ticket) in [
        ("edit.copy", "p1-parity/20"),
        ("edit.paste", "p1-parity/20"),
        ("timeline.duplicate", "p1-parity/16"),
        ("timeline.split_left", "p1-parity/16"),
        ("timeline.split_right", "p1-parity/16"),
    ] {
        assert!(matches!(
            editor.activate(action),
            Err(vibecut::Error::NotAvailable(owner)) if owner == ticket
        ));
    }
    assert_eq!(editor.session.project(), &project);
}

#[test]
fn binding_changes_use_app_history_and_leave_the_project_untouched() {
    let fixture = fixture();
    let mut editor = fixture.editor;
    let project = editor.session.project().clone();
    let project_can_undo = editor.session.can_undo();
    editor
        .dispatch(
            actions::KEYBINDINGS_SET_COMMAND,
            serde_json::json!({
                "action": "timeline.snap",
                "bindings": ["Ctrl+Shift+N"]
            }),
        )
        .unwrap();
    assert_eq!(
        editor.preferences.keybindings.bindings["timeline.snap"],
        ["Ctrl+Shift+N"]
    );
    assert!(
        editor
            .preferences
            .keybindings
            .is_customized
            .contains("timeline.snap")
    );
    assert_eq!(editor.session.project(), &project);
    assert_eq!(editor.session.can_undo(), project_can_undo);

    editor.activate("edit.undo").unwrap();
    assert!(editor.preferences.keybindings.bindings.is_empty());
    assert!(editor.preferences.keybindings.is_customized.is_empty());
    assert_eq!(editor.session.project(), &project);

    editor.activate("edit.redo").unwrap();
    assert_eq!(
        editor.preferences.keybindings.bindings["timeline.snap"],
        ["Ctrl+Shift+N"]
    );
    assert_eq!(editor.session.project(), &project);

    editor.activate("edit.undo").unwrap();
    editor.activate("edit.undo").unwrap();
    assert!(editor.preferences.keybindings.bindings.is_empty());

    let mut harness = harness(editor);
    for bindings in [vec!["Ctrl+Shift+N".to_owned()], Vec::new()] {
        harness
            .state_mut()
            .set_keybinding("timeline.snap", bindings.clone())
            .unwrap();
        harness.run_steps(3);
        let inspection = harness.state().inspect().unwrap();
        let tooltip = inspection
            .widgets
            .iter()
            .find(|widget| widget.id == "timeline.snap")
            .unwrap()
            .tooltip
            .as_deref()
            .unwrap();
        if let Some(binding) = bindings.first() {
            let chord = actions::Chord::parse(binding)
                .unwrap()
                .display(actions::Platform::current());
            assert!(tooltip.contains(&chord), "{tooltip}");
        } else {
            assert_eq!(tooltip, "Snap");
        }
    }
}

#[test]
fn unknown_actions_are_reported() {
    let fixture = fixture();
    let mut editor = fixture.editor;
    editor
        .dispatch(
            PREFERENCE_COMMAND,
            serde_json::json!({
                "key": "keybindings",
                "value": {
                    "bindings": {"future.action": ["Ctrl+K"]},
                    "isCustomized": ["future.action"]
                }
            }),
        )
        .unwrap();
    assert!(
        editor
            .inspect()
            .unwrap()
            .keybinding_issues
            .iter()
            .any(|issue| issue.to_string().contains("future.action"))
    );
}

#[test]
fn shift_arrows_and_j_l_both_seek_one_second() {
    let fixture = fixture();
    let mut editor = fixture.editor;
    editor
        .playback
        .seek(Tick(3 * SECOND), false, editor.session.project().clone())
        .unwrap();
    let mut harness = harness(editor);
    harness.run_steps(3);
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
    harness.run_steps(2);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(4 * SECOND)
    );
    harness.key_press(egui::Key::J);
    harness.run_steps(2);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(3 * SECOND)
    );
}

#[test]
fn registered_delete_target_is_executed_by_keys_buttons_and_direct_dispatch() {
    for ripple in [false, true] {
        for entry in ["key", "button", "dispatch"] {
            let fixture = fixture();
            let item = fixture.video_a;
            let mut editor = fixture.editor;
            editor.selection.insert(item);
            editor.ripple = ripple;
            let actions::ActionTarget::Engine(route) =
                actions::action("timeline.delete").unwrap().target
            else {
                panic!("delete must be an engine route");
            };
            let command = route.command(ripple);
            let mut expected =
                engine::Session::new((**editor.session.project()).clone(), 50).unwrap();
            let params = if ripple {
                serde_json::json!({"item":item})
            } else {
                serde_json::json!({"items":[item]})
            };
            expected.execute(command.id(), params).unwrap();
            let mut h = harness(editor);
            h.run_steps(3);
            match entry {
                "key" => h.key_press(egui::Key::Delete),
                "button" => h
                    .get(By::new().predicate(|n| n.author_id() == Some("timeline.delete")))
                    .click(),
                "dispatch" => h
                    .state_mut()
                    .dispatch("timeline.delete", serde_json::json!({}))
                    .unwrap(),
                _ => panic!("invalid test action entry"),
            }
            h.run_steps(3);
            assert_eq!(
                h.state().session.project(),
                expected.project(),
                "{entry}, ripple={ripple}"
            );
            assert!(h.state().selection.is_empty());
        }
    }
}

#[test]
fn disabled_engine_and_app_selection_states_refuse_every_action_entry() {
    for locked in [false, true] {
        let fixture = fixture();
        let mut editor = fixture.editor;
        if locked {
            editor.selection.insert(fixture.video_a);
            let tracks: Vec<_> = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .map(|track| track.id)
                .collect();
            for track in tracks {
                editor
                    .session
                    .apply(engine::Command::SetTrackLocked {
                        track,
                        locked: true,
                    })
                    .unwrap();
            }
            assert!(!engine::CommandKind::Delete.enabled(&editor.session));
        } else {
            assert!(engine::CommandKind::Delete.enabled(&editor.session));
        }
        assert!(!editor.action_enabled("timeline.delete"));
        let before = editor.session.project().clone();
        assert!(matches!(
            editor.activate("timeline.delete"),
            Err(vibecut::Error::ActionDisabled("timeline.delete"))
        ));
        assert!(
            editor
                .dispatch("timeline.delete", serde_json::json!({}))
                .is_err()
        );
        let mut h = harness(editor);
        h.run_steps(3);
        assert!(
            !h.state()
                .inspect()
                .unwrap()
                .widgets
                .iter()
                .find(|w| w.id == "timeline.delete")
                .unwrap()
                .enabled
        );
        h.key_press(egui::Key::Delete);
        h.run_steps(3);
        assert_eq!(h.state().session.project(), &before);
    }
}
