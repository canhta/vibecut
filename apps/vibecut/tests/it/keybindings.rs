#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
use super::timeline::{fixture, harness, widget};
use egui_kittest::kittest::{By, Queryable};
use engine::Command;
use time::{TICKS_PER_SECOND, Tick};
use vibecut::{
    AppCommand, Editor, ShortcutTab, actions, egui,
    i18n::{self, Locale},
    preferences::PreferenceStore,
};

const SECOND: i64 = TICKS_PER_SECOND;

#[test]
fn default_split_chord_mutates_history_and_undo_redo_restore_it() {
    let fixture = fixture();
    let id = fixture.video_a;
    let mut editor = fixture.editor;
    editor.select([id].into()).unwrap();
    editor
        .playback
        .seek(Tick(3 * SECOND), false, editor.session.project().clone())
        .unwrap();
    let before = editor.session.project().clone();
    let mut harness = harness(editor);
    harness.run_steps(3);

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
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
    editor.select([id].into()).unwrap();
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
    assert_eq!(*harness.state().selection(), [id].into_iter().collect());
    assert!(!harness.state().session.gesture_active());
    assert_eq!(harness.state().session.project(), &before);

    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(harness.state().selection().is_empty());
    harness.drop_at(from);
    harness.run_steps(2);
}

#[test]
fn clipboard_actions_refuse_empty_scopes_without_mutating_project_or_history() {
    let mut editor = fixture().editor;
    let project = editor.session.project().clone();
    let revision = editor.session.revision();
    let history = editor.session.history_len();
    for action in [
        "edit.copy",
        "edit.cut",
        "edit.paste",
        "edit.copy_attributes",
        "edit.paste_attributes",
    ] {
        assert!(
            matches!(editor.activate(action),Err(vibecut::Error::ActionDisabled(id)) if id==action)
        );
    }
    assert_eq!(editor.session.project(), &project);
    assert_eq!(editor.session.revision(), revision);
    assert_eq!(editor.session.history_len(), history);
}

#[test]
fn copy_and_paste_attributes_chords_copy_audio_groups_and_name_the_skipped_ones() {
    let fixture = fixture();
    let video = fixture.video_a;
    let mut editor = fixture.editor;
    let sound = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .audio
        .iter()
        .flat_map(|track| track.items.iter())
        .map(|item| item.id)
        .next()
        .unwrap();
    editor
        .session
        .apply(Command::SetFade {
            item: video,
            edge: project::FadeEdge::In,
            duration: Tick(SECOND),
            curve: project::FadeCurve::Linear,
        })
        .unwrap();
    let mut harness = harness(editor);
    harness.run_steps(2);
    let both = egui::Modifiers {
        shift: true,
        ..egui::Modifiers::COMMAND
    };
    harness
        .state_mut()
        .session
        .set_selection([video].into())
        .unwrap();
    harness.run_steps(2);
    harness.key_press_modifiers(both, egui::Key::C);
    harness.run_steps(2);
    assert!(harness.state().session.clipboard_has_entry());
    harness
        .state_mut()
        .session
        .set_selection([sound].into())
        .unwrap();
    harness.run_steps(2);
    let before = harness.state().session.project().clone();
    let history = harness.state().session.history_len().0;
    harness.key_press_modifiers(both, egui::Key::V);
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history + 1);
    let fades_of = |editor: &Editor, id| match &super::timeline::item(editor, id).content {
        project::ItemParams::Video(p) => p.fades,
        project::ItemParams::Audio(p) => p.fades,
        _ => panic!("media item"),
    };
    let state = harness.state();
    assert_eq!(fades_of(state, sound), fades_of(state, video));
    assert_ne!(fades_of(state, sound), project::Fades::default());
    let toast = state.inspect().unwrap().import_toast.unwrap();
    for group in ["Transform", "Blend", "Effects", "Masks"] {
        assert!(toast.contains(group), "{toast:?} missing {group}");
    }
    assert!(
        !toast.contains("Audio") && !toast.contains("Speed"),
        "{toast:?}"
    );
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(harness.state().session.project(), &before);
}

#[test]
fn binding_changes_use_app_history_and_leave_the_project_untouched() {
    let fixture = fixture();
    let mut editor = fixture.editor;
    let project = editor.session.project().clone();
    let project_can_undo = editor.session.can_undo();
    editor
        .dispatch(AppCommand::SetKeybinding {
            action: "timeline.snap".to_owned(),
            chords: vec!["Ctrl+Shift+N".to_owned()],
        })
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
        .set_preference(
            "keybindings",
            serde_json::json!({
                "active": 0,
                "profiles": [
                    {"bindings": {"future.action": ["Ctrl+K"]}, "isCustomized": ["future.action"]},
                    {"bindings": {}, "isCustomized": []},
                    {"bindings": {}, "isCustomized": []},
                    {"bindings": {}, "isCustomized": []},
                    {"bindings": {}, "isCustomized": []}
                ]
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
fn bookmark_jump_dispatch_shares_the_playhead_without_project_or_history_change() {
    let fixture = fixture();
    let id = fixture.video_a;
    let mut editor = fixture.editor;
    editor
        .session
        .apply(Command::BookmarkAdd {
            start: Tick(3 * SECOND),
            duration: Tick::ZERO,
            note: "Early".into(),
            color: project::Color([255, 255, 0, 255]),
        })
        .unwrap();
    let selection: std::collections::BTreeSet<_> = [id].into_iter().collect();
    editor.select(selection.clone()).unwrap();
    let project = serde_json::to_vec(editor.session.project().as_ref()).unwrap();
    let can_undo = editor.session.can_undo();
    assert!(editor.action_enabled("playback.jump_bookmark.next"));
    assert!(!editor.action_enabled("playback.jump_bookmark.previous"));

    editor
        .dispatch(AppCommand::JumpBookmark(actions::BookmarkDirection::Next))
        .unwrap();
    assert_eq!(editor.playback.position().unwrap(), Tick(3 * SECOND));
    assert_eq!(editor.inspect().unwrap().position, Tick(3 * SECOND));
    editor
        .dispatch(AppCommand::JumpBookmark(actions::BookmarkDirection::Next))
        .unwrap();
    assert_eq!(editor.playback.position().unwrap(), Tick(8 * SECOND));

    let generation = editor.playback.generation();
    let view = editor.session.metadata().view;
    editor
        .dispatch(AppCommand::JumpBookmark(actions::BookmarkDirection::Next))
        .unwrap();
    assert_eq!(editor.playback.position().unwrap(), Tick(8 * SECOND));
    assert_eq!(
        editor.playback.generation(),
        generation,
        "no seek at the end"
    );
    assert_eq!(editor.session.metadata().view, view, "no metadata write");

    editor
        .dispatch(AppCommand::JumpBookmark(
            actions::BookmarkDirection::Previous,
        ))
        .unwrap();
    assert_eq!(editor.playback.position().unwrap(), Tick(3 * SECOND));
    editor
        .dispatch(AppCommand::JumpBookmark(
            actions::BookmarkDirection::Previous,
        ))
        .unwrap();
    assert_eq!(editor.playback.position().unwrap(), Tick(3 * SECOND));

    assert_eq!(*editor.selection(), selection);
    assert_eq!(editor.session.can_undo(), can_undo);
    assert_eq!(
        serde_json::to_vec(editor.session.project().as_ref()).unwrap(),
        project
    );
}

#[test]
fn bookmark_jump_bindings_persist_and_run_through_the_normal_keymap() {
    let fixture = fixture();
    let mut editor = fixture.editor;
    editor
        .session
        .apply(Command::BookmarkAdd {
            start: Tick(3 * SECOND),
            duration: Tick::ZERO,
            note: "Early".into(),
            color: project::Color([255, 255, 0, 255]),
        })
        .unwrap();
    editor
        .record_shortcut("playback.jump_bookmark.next", "Ctrl+Alt+Right")
        .unwrap();
    editor
        .record_shortcut("playback.jump_bookmark.previous", "Ctrl+Alt+Left")
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    store.save(&editor.preferences).unwrap();
    let loaded = store.load_with_warning().0;
    editor
        .set_preference(
            "keybindings",
            serde_json::to_value(loaded.keybindings).unwrap(),
        )
        .unwrap();

    let mut harness = harness(editor);
    harness.run_steps(3);
    let chord = egui::Modifiers::COMMAND.plus(egui::Modifiers::ALT);
    harness.state_mut().activate("transport.play").unwrap();
    harness.run_steps(2);
    assert!(harness.state().playback.playing());
    harness.key_press_modifiers(chord, egui::Key::ArrowRight);
    harness.run_steps(1);
    assert!(harness.state().playback.playing(), "jump keeps playing");
    assert!(
        harness.state().playback.position().unwrap() >= Tick(3 * SECOND),
        "a playing jump reaches the next bookmark, at {:?}",
        harness.state().playback.position().unwrap()
    );
    harness.state_mut().activate("transport.pause").unwrap();
    harness.run_steps(2);

    harness
        .state_mut()
        .dispatch(AppCommand::Seek(Tick::ZERO))
        .unwrap();
    harness.run_steps(1);
    harness.key_press_modifiers(chord, egui::Key::ArrowRight);
    harness.run_steps(2);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(3 * SECOND)
    );
    harness.key_press_modifiers(chord, egui::Key::ArrowRight);
    harness.run_steps(2);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(8 * SECOND)
    );
    harness.key_press_modifiers(chord, egui::Key::ArrowRight);
    harness.run_steps(2);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(8 * SECOND),
        "no wrap at the last bookmark"
    );
    harness.key_press_modifiers(chord, egui::Key::ArrowLeft);
    harness.run_steps(2);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(3 * SECOND)
    );

    harness
        .get(By::new().predicate(|node| node.author_id() == Some("preview.timecode")))
        .click();
    harness.run_steps(2);
    harness.key_press_modifiers(chord, egui::Key::ArrowRight);
    harness.run_steps(2);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(3 * SECOND),
        "a focused text field still blocks the transport chord"
    );
}

#[test]
fn registered_delete_target_is_executed_by_keys_buttons_and_direct_activation() {
    for ripple in [false, true] {
        for entry in ["key", "button", "dispatch"] {
            let fixture = fixture();
            let item = fixture.video_a;
            let mut editor = fixture.editor;
            editor.select([item].into()).unwrap();
            editor.preferences.ripple = ripple;
            let actions::ActionTarget::Engine(route) =
                actions::action("timeline.delete").unwrap().target
            else {
                panic!("delete must be an engine route");
            };
            let command = route.command(ripple);
            let mut expected =
                engine::Session::new((**editor.session.project()).clone(), 50).unwrap();
            let expected_command = match command {
                engine::CommandKind::Delete => Command::Delete { items: vec![item] },
                engine::CommandKind::RippleDelete => Command::RippleDelete { item },
                other => panic!("delete route must build delete or ripple delete: {other:?}"),
            };
            expected.apply(expected_command).unwrap();
            let mut h = harness(editor);
            h.run_steps(3);
            match entry {
                "key" => h.key_press(egui::Key::Delete),
                "button" => h
                    .get(By::new().predicate(|n| n.author_id() == Some("timeline.delete")))
                    .click(),
                "dispatch" => h.state_mut().activate("timeline.delete").unwrap(),
                _ => panic!("invalid test action entry"),
            }
            h.run_steps(3);
            assert_eq!(
                h.state().session.project(),
                expected.project(),
                "{entry}, ripple={ripple}"
            );
            assert!(h.state().selection().is_empty());
        }
    }
}

#[test]
fn disabled_engine_and_app_selection_states_refuse_every_action_entry() {
    for locked in [false, true] {
        let fixture = fixture();
        let mut editor = fixture.editor;
        if locked {
            editor.select([fixture.video_a].into()).unwrap();
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
        assert!(editor.activate("timeline.delete").is_err());
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

fn click(harness: &mut egui_kittest::Harness<'_, Editor>, id: &str) {
    let pos = widget(harness, id).center();
    harness.drag_at(pos);
    harness.run_steps(1);
    harness.drop_at(pos);
    harness.run_steps(3);
}

fn open_shortcuts(harness: &mut egui_kittest::Harness<'_, Editor>) {
    click(harness, "shortcuts.open");
    assert!(
        harness.state().inspect().unwrap().shortcuts.is_some(),
        "header button must open the dialog"
    );
}

#[test]
fn shortcut_rows_and_conflict_messages_use_the_vietnamese_catalog() {
    let mut fixture = fixture();
    fixture.editor.preferences.locale = Locale::Vi;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    open_shortcuts(&mut harness);
    let rows = harness.state().inspect().unwrap().shortcuts.unwrap().rows;
    let split = rows
        .iter()
        .find(|row| row.action == "timeline.split")
        .expect("split row");
    assert_eq!(
        split.label,
        i18n::text(Locale::Vi, actions::action("timeline.split").unwrap().label),
        "the shortcut list must show the Vietnamese action label"
    );
    assert!(
        !split.label.is_ascii(),
        "the Vietnamese label must not fall back to English"
    );

    // Every registered action carries its own catalog key with both rows.
    let mut keys = std::collections::BTreeSet::new();
    for spec in actions::actions() {
        assert!(
            keys.insert(spec.label.key()),
            "{} reuses catalog key {}",
            spec.id,
            spec.label.key()
        );
        assert!(!spec.label.text(Locale::En).is_empty(), "{} en", spec.id);
        assert!(!spec.label.text(Locale::Vi).is_empty(), "{} vi", spec.id);
    }

    // A conflict prompt names the owning action; its label is localized in the prompt.
    harness
        .state_mut()
        .record_shortcut("timeline.split", "N")
        .unwrap();
    let shortcuts = harness.state().inspect().unwrap().shortcuts.unwrap();
    let conflict = shortcuts.conflict.expect("conflict prompt");
    assert_eq!(conflict.owner, "timeline.snap");
    assert_eq!(conflict.chord, "N");
    let owner_label = i18n::text(Locale::Vi, actions::action("timeline.snap").unwrap().label);
    assert!(
        i18n::format(
            Locale::Vi,
            i18n::Text::ShortcutsConflictBody,
            &[("chord", "N"), ("action", owner_label)],
        )
        .contains(owner_label),
        "the Vietnamese prompt must name the localized owner"
    );
}

#[test]
fn shortcuts_dialog_records_binds_unbound_actions_and_survives_undo_redo() {
    let fixture = fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    open_shortcuts(&mut harness);
    let project = harness.state().session.project().clone();
    let project_can_undo = harness.state().session.can_undo();
    let inspection = harness.state().inspect().unwrap();
    let shortcuts = inspection.shortcuts.unwrap();
    assert_eq!(
        shortcuts.rows.len(),
        actions::actions()
            .iter()
            .filter(|spec| spec.bindable())
            .count()
    );
    for category in actions::ActionCategory::ALL {
        assert!(
            shortcuts
                .rows
                .iter()
                .any(|row| row.category == category.label()),
            "category {} must own at least one row",
            category.label()
        );
    }
    for tab in ShortcutTab::ALL.iter().copied() {
        assert!(
            shortcuts.rows.iter().any(|row| row.tab == tab),
            "tab {} must own at least one row",
            tab.id()
        );
    }
    let ripple = shortcuts
        .rows
        .iter()
        .find(|row| row.action == "timeline.ripple")
        .unwrap();
    assert!(ripple.unbound, "actions without defaults stay visible");
    assert!(ripple.keys.is_empty());
    assert!(
        shortcuts
            .rows
            .iter()
            .any(|row| row.action == "project.save_as" && row.unbound)
    );

    click(&mut harness, "shortcuts.tab.basic");
    assert_eq!(
        harness.state().inspect().unwrap().shortcuts.unwrap().tab,
        ShortcutTab::Basic
    );
    click(&mut harness, "shortcuts.row.file.import");
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::R,
    );
    harness.run_steps(3);
    let expected = actions::Chord::parse("Ctrl+Shift+R")
        .unwrap()
        .display(actions::Platform::current());
    // The chord lives in the dialog's working copy until Save.
    assert!(
        !harness
            .state()
            .preferences
            .keybindings
            .bindings
            .contains_key("file.import")
    );
    let inspection = harness.state().inspect().unwrap();
    let shortcuts = inspection.shortcuts.unwrap();
    assert!(shortcuts.recording.is_none());
    assert!(shortcuts.error.is_none());
    assert!(shortcuts.changed, "Save enables once the copy differs");
    let row = shortcuts
        .rows
        .iter()
        .find(|row| row.action == "file.import")
        .unwrap();
    assert!(!row.unbound);
    assert!(row.customized);
    assert_eq!(row.keys, [expected]);

    click(&mut harness, "shortcuts.save");
    assert!(harness.state().inspect().unwrap().shortcuts.is_none());
    assert_eq!(
        harness.state().preferences.keybindings.bindings["file.import"],
        ["Ctrl+Shift+R"]
    );
    assert!(
        harness
            .state()
            .preferences
            .keybindings
            .is_customized
            .contains("file.import")
    );

    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(3);
    assert!(harness.state().preferences.keybindings.bindings.is_empty());
    harness.state_mut().activate("edit.redo").unwrap();
    harness.run_steps(3);
    assert_eq!(
        harness.state().preferences.keybindings.bindings["file.import"],
        ["Ctrl+Shift+R"]
    );
    assert_eq!(harness.state().session.project(), &project);
    assert_eq!(harness.state().session.can_undo(), project_can_undo);
}

#[test]
fn shortcuts_dialog_prompts_on_conflicts_and_overrides_or_keeps_the_edit_box() {
    let fixture = fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    open_shortcuts(&mut harness);
    let project = harness.state().session.project().clone();
    let project_can_undo = harness.state().session.can_undo();
    let ctrl_b = actions::Chord::parse("Ctrl+B")
        .unwrap()
        .display(actions::Platform::current());
    let split = actions::action("timeline.split").unwrap();

    // Save stays disabled until the working copy differs from the stored map.
    let inspection = harness.state().inspect().unwrap();
    assert!(!inspection.shortcuts.as_ref().unwrap().changed);
    assert!(
        inspection
            .widgets
            .iter()
            .any(|widget| widget.id == "shortcuts.save" && !widget.enabled),
        "Save must be disabled before a change"
    );

    // Ctrl+B already belongs to Split at playhead: the prompt names it and the box keeps recording.
    click(&mut harness, "shortcuts.row.timeline.snap");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.run_steps(3);
    let shortcuts = harness.state().inspect().unwrap().shortcuts.unwrap();
    let conflict = shortcuts.conflict.clone().expect("conflict prompt");
    assert_eq!(conflict.action, "timeline.snap");
    assert_eq!(conflict.chord, "Ctrl+B");
    assert_eq!(conflict.owner, "timeline.split");
    assert_eq!(shortcuts.recording.as_deref(), Some("timeline.snap"));
    assert!(
        !harness
            .state()
            .preferences
            .keybindings
            .bindings
            .contains_key("timeline.snap")
    );
    assert_eq!(
        harness
            .state()
            .preferences
            .keybindings
            .effective_chords(split),
        ["Ctrl+B"]
    );

    // Cancel keeps the edit box open and changes nothing.
    click(&mut harness, "shortcuts.conflict.cancel");
    let shortcuts = harness.state().inspect().unwrap().shortcuts.unwrap();
    assert!(shortcuts.conflict.is_none());
    assert_eq!(shortcuts.recording.as_deref(), Some("timeline.snap"));
    assert!(!shortcuts.changed);

    // Override moves Ctrl+B to snapping; Split loses it and its row turns unbound.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.run_steps(3);
    click(&mut harness, "shortcuts.conflict.override");
    let shortcuts = harness.state().inspect().unwrap().shortcuts.unwrap();
    assert!(shortcuts.conflict.is_none());
    assert!(shortcuts.recording.is_none());
    assert!(shortcuts.error.is_none());
    assert!(shortcuts.changed);
    let row = |id: &str| shortcuts.rows.iter().find(|row| row.action == id).unwrap();
    assert_eq!(row("timeline.snap").keys, std::slice::from_ref(&ctrl_b));
    assert!(row("timeline.split").unbound);
    assert_eq!(harness.state().session.project(), &project);

    // Escape inside the edit box is recorded as a key and conflicts with Cancel or deselect.
    click(&mut harness, "shortcuts.row.timeline.snap");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    let shortcuts = harness.state().inspect().unwrap().shortcuts.unwrap();
    let conflict = shortcuts.conflict.clone().expect("Escape conflict prompt");
    assert_eq!(conflict.chord, "Escape");
    assert_eq!(conflict.owner, "selection.escape");
    assert_eq!(shortcuts.recording.as_deref(), Some("timeline.snap"));
    click(&mut harness, "shortcuts.conflict.cancel");
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .shortcuts
            .unwrap()
            .conflict
            .is_none()
    );

    // A chord the parser refuses reports the reason and keeps the edit box open.
    let invalid = harness
        .state_mut()
        .record_shortcut("timeline.snap", "Ctrl+NotAKey")
        .unwrap_err();
    assert!(
        matches!(
            invalid,
            vibecut::Error::Keybinding(actions::KeybindingError::InvalidChord { .. })
        ),
        "{invalid:?}"
    );
    harness.run_steps(3);
    let shortcuts = harness.state().inspect().unwrap().shortcuts.unwrap();
    let error = shortcuts.error.as_deref().unwrap();
    assert!(error.contains("Can't use"), "{error}");
    assert!(error.contains("NotAKey"), "{error}");
    assert_eq!(shortcuts.recording.as_deref(), Some("timeline.snap"));

    // A second click on the recording row ends its edit box; Escape then closes the dialog.
    click(&mut harness, "shortcuts.row.timeline.snap");
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .shortcuts
            .unwrap()
            .recording
            .is_none()
    );
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(harness.state().inspect().unwrap().shortcuts.is_none());
    assert!(harness.state().preferences.keybindings.bindings.is_empty());
    assert_eq!(harness.state().session.project(), &project);
    assert_eq!(harness.state().session.can_undo(), project_can_undo);
}

#[test]
fn shortcuts_dialog_resets_one_and_all_actions_and_undo_restores_the_map() {
    let fixture = fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    open_shortcuts(&mut harness);
    let project = harness.state().session.project().clone();
    let project_can_undo = harness.state().session.can_undo();
    for (action, chord) in [
        ("timeline.snap", "Ctrl+Shift+N"),
        ("timeline.ripple", "Ctrl+Shift+R"),
    ] {
        harness.state_mut().record_shortcut(action, chord).unwrap();
    }
    harness.run_steps(3);
    click(&mut harness, "shortcuts.save");
    let custom = harness.state().preferences.keybindings.clone();
    assert_eq!(custom.bindings.len(), 2);
    assert_eq!(custom.is_customized.len(), 2);

    // Reset edits the working copy; Save commits it as one preference edit.
    open_shortcuts(&mut harness);
    click(&mut harness, "shortcuts.reset.timeline.snap");
    assert_eq!(harness.state().preferences.keybindings, custom);
    click(&mut harness, "shortcuts.save");
    let reset_one = harness.state().preferences.keybindings.clone();
    assert!(!reset_one.bindings.contains_key("timeline.snap"));
    assert!(!reset_one.is_customized.contains("timeline.snap"));
    assert!(reset_one.bindings.contains_key("timeline.ripple"));
    assert!(reset_one.is_customized.contains("timeline.ripple"));

    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.keybindings, custom);
    harness.state_mut().activate("edit.redo").unwrap();
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.keybindings, reset_one);

    // Reset all has no confirmation: it clears the working copy at once, and Save commits.
    open_shortcuts(&mut harness);
    click(&mut harness, "shortcuts.reset_all");
    let inspection = harness.state().inspect().unwrap().shortcuts.unwrap();
    assert!(inspection.conflict.is_none());
    assert!(inspection.changed);
    assert_eq!(harness.state().preferences.keybindings, reset_one);
    click(&mut harness, "shortcuts.save");
    let reset_all = harness.state().preferences.keybindings.clone();
    assert!(reset_all.bindings.is_empty());
    assert!(reset_all.is_customized.is_empty());
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.keybindings, reset_one);
    assert_eq!(harness.state().session.project(), &project);
    assert_eq!(harness.state().session.can_undo(), project_can_undo);
}
#[test]
fn shortcuts_bindings_and_reset_state_roundtrip_through_the_preference_store() {
    let fixture = fixture();
    let mut editor = fixture.editor;
    let project = editor.session.project().clone();
    let project_can_undo = editor.session.can_undo();
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));

    editor
        .record_shortcut("timeline.snap", "Ctrl+Shift+N")
        .unwrap();
    editor
        .record_shortcut("timeline.ripple", "Ctrl+Shift+R")
        .unwrap();
    store.save(&editor.preferences).unwrap();
    let loaded = store.load_with_warning().0;
    assert_eq!(
        loaded.keybindings.bindings["timeline.snap"],
        ["Ctrl+Shift+N"]
    );
    assert_eq!(
        loaded.keybindings.bindings["timeline.ripple"],
        ["Ctrl+Shift+R"]
    );
    assert!(loaded.keybindings.is_customized.contains("timeline.snap"));
    let snap = actions::action("timeline.snap").unwrap();
    assert_eq!(loaded.keybindings.effective_chords(snap), ["Ctrl+Shift+N"]);

    editor
        .dispatch(AppCommand::ResetKeybindings {
            action: Some("timeline.snap".to_owned()),
        })
        .unwrap();
    editor
        .dispatch(AppCommand::ResetKeybindings { action: None })
        .unwrap();
    store.save(&editor.preferences).unwrap();
    let reloaded = store.load_with_warning().0;
    assert!(reloaded.keybindings.bindings.is_empty());
    assert!(reloaded.keybindings.is_customized.is_empty());
    assert_eq!(reloaded.keybindings.effective_chords(snap), ["N"]);

    // Recording the registry default removes both the override and marker.
    editor.record_shortcut("timeline.snap", "N").unwrap();
    assert!(
        !editor
            .preferences
            .keybindings
            .bindings
            .contains_key("timeline.snap")
    );
    assert!(
        !editor
            .preferences
            .keybindings
            .is_customized
            .contains("timeline.snap")
    );
    assert_eq!(editor.session.project(), &project);
    assert_eq!(editor.session.can_undo(), project_can_undo);
}

#[test]
fn shortcuts_import_export_entry_points_are_disabled_until_settings_export() {
    let fixture = fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    open_shortcuts(&mut harness);
    let inspection = harness.state().inspect().unwrap();
    for id in [
        "shortcuts.import",
        "shortcuts.export",
        "shortcuts.preset",
        "shortcuts.save",
        "shortcuts.cancel",
        "shortcuts.close",
    ] {
        assert!(
            inspection.widgets.iter().any(|widget| widget.id == id),
            "missing dialog control {id}"
        );
    }
    for id in ["shortcuts.import", "shortcuts.export"] {
        let widget = inspection
            .widgets
            .iter()
            .find(|widget| widget.id == id)
            .unwrap_or_else(|| panic!("missing disabled entry point {id}"));
        assert!(!widget.enabled, "{id} stays disabled until settings export");
    }
}

#[test]
fn shortcut_presets_keep_their_own_bindings_and_persist_the_choice() {
    let fixture = fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let one = actions::KeybindingPreset::One;
    let two = actions::KeybindingPreset::Two;
    let snap = actions::action("timeline.snap").unwrap();
    let split = actions::action("timeline.split").unwrap();
    let ctrl_shift_n = actions::Chord::parse("Ctrl+Shift+N")
        .unwrap()
        .display(actions::Platform::current());
    let project = harness.state().session.project().clone();

    // Preset 1: record a chord and save it into the active profile.
    open_shortcuts(&mut harness);
    click(&mut harness, "shortcuts.row.timeline.snap");
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::N,
    );
    harness.run_steps(3);
    click(&mut harness, "shortcuts.save");
    let keybindings = &harness.state().preferences.keybindings;
    assert_eq!(keybindings.active, one);
    assert_eq!(
        keybindings.effective_chords(snap),
        std::slice::from_ref(&ctrl_shift_n)
    );

    // Switching to preset 2 applies its own profile: snapping is back on N there.
    harness.state_mut().select_shortcut_preset(two).unwrap();
    harness.run_steps(3);
    let keybindings = &harness.state().preferences.keybindings;
    assert_eq!(keybindings.active, two);
    assert_eq!(keybindings.effective_chords(snap), ["N"]);
    assert_eq!(
        keybindings.profile(one).effective_chords(snap),
        std::slice::from_ref(&ctrl_shift_n),
        "preset 1 keeps its chord while preset 2 is active"
    );

    // An override in preset 2 moves Ctrl+B to snapping there and leaves preset 1 alone.
    open_shortcuts(&mut harness);
    assert_eq!(
        harness.state().inspect().unwrap().shortcuts.unwrap().preset,
        two
    );
    click(&mut harness, "shortcuts.row.timeline.snap");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.run_steps(3);
    click(&mut harness, "shortcuts.conflict.override");
    click(&mut harness, "shortcuts.save");
    let keybindings = &harness.state().preferences.keybindings;
    assert_eq!(
        keybindings.effective_chords(snap),
        ["Ctrl+B"].map(String::from)
    );
    assert!(keybindings.effective_chords(split).is_empty());
    assert_eq!(
        keybindings.profile(one).effective_chords(snap),
        std::slice::from_ref(&ctrl_shift_n)
    );
    assert_eq!(keybindings.profile(one).effective_chords(split), ["Ctrl+B"]);
    assert_eq!(harness.state().session.project(), &project);

    // The active choice and every profile survive a store round-trip.
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    store.save(&harness.state().preferences).unwrap();
    let loaded = store.load_with_warning().0;
    assert_eq!(loaded.keybindings, harness.state().preferences.keybindings);
    assert_eq!(loaded.keybindings.active, two);
    assert_eq!(
        loaded.keybindings.profile(one).effective_chords(snap),
        [ctrl_shift_n]
    );
}

#[test]
fn shortcuts_dialog_tabs_switch_and_capture_every_tab_at_two_x() {
    let fixture = fixture();
    let mut editor = fixture.editor;
    editor.open_shortcuts();
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_pixels_per_point(2.0)
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(3);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&root).unwrap();
    let mut rects = serde_json::Map::new();
    for tab in ShortcutTab::ALL.iter().copied() {
        if tab != ShortcutTab::Timeline {
            click(&mut harness, &format!("shortcuts.tab.{}", tab.id()));
        }
        harness.run_steps(12);
        assert_eq!(
            harness.state().inspect().unwrap().shortcuts.unwrap().tab,
            tab
        );
        let rows = harness.state().inspect().unwrap().shortcuts.unwrap().rows;
        println!(
            "tab {} rows {}",
            tab.id(),
            rows.iter().filter(|row| row.tab == tab).count()
        );
        let dialog = widget(&harness, "shortcuts.dialog");
        rects.insert(
            tab.id().to_owned(),
            serde_json::json!([dialog.min.x, dialog.min.y, dialog.width(), dialog.height()]),
        );
        let path = root.join(format!("p1-04-shortcuts-{}.png", tab.id()));
        harness.render().unwrap().save(&path).unwrap();
        println!("shortcuts {tab:?} screenshot: {}", path.display());
    }
    std::fs::copy(
        root.join("p1-04-shortcuts-timeline.png"),
        root.join("p1-04-shortcuts.png"),
    )
    .unwrap();
    std::fs::write(
        root.join("p1-04-shortcuts-rects.json"),
        serde_json::json!({"scale": 2.0, "tabs": rects}).to_string(),
    )
    .unwrap();
    println!(
        "shortcuts screenshot: {}",
        root.join("p1-04-shortcuts.png").display()
    );
}

#[test]
fn reference_default_chords_run_their_actions_through_the_keymap() {
    // One row per adopted group. Each chord goes through the keymap the frame
    // dispatches with, so a collision or a missing registry row fails here.
    let keymap = actions::Keymap::resolve(&actions::KeybindingPreferences::default());
    let command = egui::Modifiers::COMMAND;
    let alt_shift = egui::Modifiers::ALT.plus(egui::Modifiers::SHIFT);
    type Chords<'a> = &'a [(egui::Modifiers, egui::Key, &'a str)];
    let groups: [(&str, Chords); 7] = [
        (
            "extract audio",
            &[(
                command.plus(egui::Modifiers::SHIFT),
                egui::Key::S,
                "timeline.extract_audio",
            )],
        ),
        (
            "timeline zoom",
            &[
                (command, egui::Key::Equals, "timeline.zoom_in"),
                (command, egui::Key::Plus, "timeline.zoom_in"),
                (command, egui::Key::Minus, "timeline.zoom_out"),
            ],
        ),
        (
            "deselect",
            &[(egui::Modifiers::ALT, egui::Key::X, "timeline.deselect_all")],
        ),
        (
            "full screen",
            &[(
                command.plus(egui::Modifiers::SHIFT),
                egui::Key::F,
                "preview.fullscreen",
            )],
        ),
        (
            "player zoom",
            &[
                (alt_shift, egui::Key::Plus, "preview.zoom.in"),
                (alt_shift, egui::Key::Minus, "preview.zoom.out"),
                (alt_shift, egui::Key::Z, "preview.zoom.fit"),
            ],
        ),
        (
            "import and export",
            &[
                (command, egui::Key::I, "file.import"),
                (command, egui::Key::E, "file.export"),
            ],
        ),
        (
            "bookmarks",
            &[
                (
                    egui::Modifiers::SHIFT,
                    egui::Key::M,
                    "playback.jump_bookmark.next",
                ),
                (alt_shift, egui::Key::M, "playback.jump_bookmark.previous"),
            ],
        ),
    ];
    for (group, chords) in groups {
        for &(modifiers, key, action) in chords {
            let mut input = egui::InputState::default();
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
            let outcome = keymap.take(&mut input, actions::DispatchContext::default());
            assert!(
                matches!(outcome, actions::Dispatch::Action { action: found, .. } if found == action),
                "{group}: {key:?} should run {action}, got {outcome:?}"
            );
        }
    }
}
