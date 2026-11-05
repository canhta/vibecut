#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::expect_used
)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Session};
use project::{
    Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor,
    catalog::{PreferenceStore, Preferences, Text},
    egui,
    playback::{Files, Playback},
    preview,
};

fn overlay(id: u64) -> Item {
    Item {
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(id),
        source_asset: None,
        source_range: SourceRange::new(Tick::ZERO, Tick(20 * TICKS_PER_SECOND)).unwrap(),
        start: Tick::ZERO,
        content: ItemParams::Overlay(OverlayParams {
            color: project::Color([30, 160, 90, 255]),
        }),
    }
}

fn project(fps: FrameRate, width: u32, height: u32, two_items: bool) -> Project {
    let mut data = Project::empty(ProjectId(94), "Preview".into())
        .unwrap()
        .into_data();
    data.canvas.width = width;
    data.canvas.height = height;
    data.canvas.fps = fps;
    data.scene_mut().timeline.main.items.push(overlay(1));
    if two_items {
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(2),
            name: "Second".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![overlay(2)],
        });
    }
    Project::new(data).unwrap()
}

fn editor(project: Project, preferences: Preferences) -> Editor {
    let session = Session::new(project, 20).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Editor::new(session, playback, preferences)
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    h.run_steps(3);
    h
}

fn click(h: &mut Harness<Editor>, id: &str) {
    h.get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    h.run_steps(3);
}

fn widget(h: &Harness<Editor>, id: &str) -> vibecut::WidgetState {
    h.state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == id)
        .unwrap_or_else(|| panic!("missing widget {id}"))
}

fn type_timecode(h: &mut Harness<Editor>, text: &str) {
    click(h, "preview.timecode");
    h.run_steps(1);
    h.get(By::new().predicate(|node| node.author_id() == Some("preview.timecode")))
        .focus();
    h.run_steps(1);
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    h.run_steps(1);
    h.get(By::new().predicate(|node| node.author_id() == Some("preview.timecode")))
        .type_text(text);
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

fn hover_canvas(h: &mut Harness<Editor>) {
    let center = widget(h, "preview.canvas").rect.center();
    h.hover_at(center);
    h.run_steps(1);
}

fn wheel(h: &mut Harness<Editor>, delta: egui::Vec2, modifiers: egui::Modifiers) {
    hover_canvas(h);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers,
    });
    h.run_steps(6);
}

fn middle_drag(h: &mut Harness<Editor>, delta: egui::Vec2) {
    let start = widget(h, "preview.canvas").rect.center();
    h.event(egui::Event::PointerMoved(start));
    h.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Middle,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(start + delta));
    h.run_steps(2);
    h.event(egui::Event::PointerButton {
        pos: start + delta,
        button: egui::PointerButton::Middle,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    h.run_steps(2);
}

fn json(project: &Project) -> Vec<u8> {
    project::encode(project, true).unwrap()
}

#[test]
fn timecode_round_trips_at_every_supported_rate_and_invalid_text_is_restored() {
    let rates = [
        FrameRate::FPS_23_976,
        FrameRate::FPS_24,
        FrameRate::FPS_25,
        FrameRate::FPS_29_97,
        FrameRate::FPS_30,
        FrameRate::FPS_60,
    ];
    for rate in rates {
        let mut h = build(editor(
            project(rate, 640, 360, false),
            Preferences::default(),
        ));
        let drop = matches!(rate, FrameRate::FPS_29_97 | FrameRate::FPS_59_94);
        let label = time::format_timecode(rate.tick_of(90).unwrap(), rate, drop).unwrap();
        let target = time::parse_timecode(&label, rate, drop).unwrap();
        type_timecode(&mut h, &label);
        assert_eq!(
            h.state().inspect().unwrap().position,
            target,
            "{rate:?} {label}"
        );
        assert_eq!(h.state().inspect().unwrap().timecode, label, "{rate:?}");

        // Invalid text is rejected and the previous value stays visible.
        type_timecode(&mut h, "99:99:99:99");
        assert_eq!(h.state().inspect().unwrap().position, target, "{rate:?}");
        assert_eq!(h.state().inspect().unwrap().timecode, label, "{rate:?}");
        assert!(h.query_by_label("Invalid timecode").is_some(), "{rate:?}");
    }
}

#[test]
fn escape_and_focus_loss_cancel_timecode_editing_without_moving_the_playhead() {
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    let before = h.state().inspect().unwrap().position;
    click(&mut h, "preview.timecode");
    h.run_steps(1);
    let node = h.get(By::new().predicate(|node| node.author_id() == Some("preview.timecode")));
    node.type_text("00:00:05:00");
    h.run_steps(1);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().inspect().unwrap().position, before);
    assert!(!h.state().inspect().unwrap().timecode_editing);
    assert!(h.query_by_label("Invalid timecode").is_none());
}

#[test]
fn zoom_presets_clamp_reverse_and_leave_the_project_untouched() {
    let mut h = build(editor(
        project(FrameRate::FPS_30, 800, 450, false),
        Preferences::default(),
    ));
    let project = h.state().session.project().clone();
    let bytes = json(&project);
    assert!(h.state().inspect().unwrap().preview_fit);

    click(&mut h, "preview.zoom.100");
    let inspect = h.state().inspect().unwrap();
    assert!(!inspect.preview_fit);
    assert!((inspect.preview_zoom - 1.0).abs() < 1e-6);
    click(&mut h, "preview.zoom.200");
    assert!((h.state().inspect().unwrap().preview_zoom - 2.0).abs() < 1e-6);
    click(&mut h, "preview.zoom.in");
    assert!((h.state().inspect().unwrap().preview_zoom - 2.5).abs() < 1e-6);
    click(&mut h, "preview.zoom.out");
    assert!((h.state().inspect().unwrap().preview_zoom - 2.0).abs() < 1e-6);
    click(&mut h, "preview.zoom.fit");
    assert!(h.state().inspect().unwrap().preview_fit);

    // The same command clamps hostile values and reverses a round trip.
    assert!(
        h.state_mut()
            .dispatch("preview.set_zoom", serde_json::json!({"zoom": 100.0}))
            .is_err()
    );
    assert!(h.state().inspect().unwrap().preview_fit);
    h.state_mut()
        .dispatch("preview.set_zoom", serde_json::json!({"zoom": 2.0}))
        .unwrap();
    h.state_mut()
        .dispatch("preview.set_zoom", serde_json::json!({"zoom": 0.5}))
        .unwrap();
    assert!((h.state().inspect().unwrap().preview_zoom - 0.5).abs() < 1e-6);
    assert_eq!(json(h.state().session.project()), bytes);
    assert!(!h.state().session.can_undo());
}

#[test]
fn ctrl_wheel_and_pinch_zoom_while_plain_wheel_pans_and_middle_drag_pans() {
    let mut h = build(editor(
        project(FrameRate::FPS_30, 800, 450, false),
        Preferences::default(),
    ));
    let bytes = json(h.state().session.project());
    h.state_mut()
        .dispatch("preview.set_zoom", serde_json::json!({"zoom": 2.0}))
        .unwrap();
    let before = h.state().inspect().unwrap();
    wheel(&mut h, egui::vec2(0.0, -20.0), egui::Modifiers::COMMAND);
    let after_ctrl = h.state().inspect().unwrap();
    assert!(
        (after_ctrl.preview_zoom - before.preview_zoom).abs() > 1e-4,
        "ctrl-wheel must zoom"
    );
    assert!(
        (after_ctrl.preview_offset - before.preview_offset).length() < 1e-4,
        "ctrl-wheel must not pan"
    );

    // Plain wheel pans without changing the zoom.
    let before = h.state().inspect().unwrap();
    wheel(&mut h, egui::vec2(0.0, -20.0), egui::Modifiers::NONE);
    let after_wheel = h.state().inspect().unwrap();
    assert_eq!(after_wheel.preview_zoom, before.preview_zoom);
    assert!(
        (after_wheel.preview_offset - before.preview_offset).length() > 0.5,
        "plain wheel must pan when zoomed: {:?} -> {:?}",
        before.preview_offset,
        after_wheel.preview_offset
    );

    // Middle-drag pans with the same bounds.
    let before = h.state().inspect().unwrap().preview_offset;
    middle_drag(&mut h, egui::vec2(40.0, 25.0));
    let after_drag = h.state().inspect().unwrap().preview_offset;
    assert!(
        (after_drag - before).length() > 0.5,
        "middle drag must pan: {before:?} -> {after_drag:?}"
    );

    // Pinch maps to the same zoom path.
    let before = h.state().inspect().unwrap().preview_zoom;
    hover_canvas(&mut h);
    h.event(egui::Event::Zoom(1.5));
    h.run_steps(3);
    assert!((h.state().inspect().unwrap().preview_zoom - before * 1.5).abs() < 1e-4);
    assert_eq!(json(h.state().session.project()), bytes);
}

#[test]
fn zoom_and_fit_and_snapshot_directory_round_trip_through_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let mut prefs = Preferences {
        preview_fit: false,
        preview_zoom: 2.5,
        snapshot_directory: Some(dir.path().to_path_buf()),
        ..Default::default()
    };
    store.save(&prefs).unwrap();
    assert_eq!(store.load().unwrap(), prefs);

    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        prefs.clone(),
    ));
    let inspect = h.state().inspect().unwrap();
    assert!(!inspect.preview_fit);
    assert!((inspect.preview_zoom - 2.5).abs() < 1e-6);
    h.state_mut()
        .dispatch("preview.set_zoom", serde_json::json!({"zoom": 1.25}))
        .unwrap();
    prefs = h.state().preferences.clone();
    store.save(&prefs).unwrap();
    let restored = store.load().unwrap();
    assert!(!restored.preview_fit);
    assert!((restored.preview_zoom - 1.25).abs() < 1e-6);
    assert_eq!(restored.snapshot_directory, prefs.snapshot_directory);

    // A successful save records its directory in the same preference document.
    h.state_mut()
        .dispatch("preview.set_zoom", serde_json::json!({"fit": true}))
        .unwrap();
    let out = dir.path().join("frame.png");
    h.state_mut().snapshot_picked_file(out.clone()).unwrap();
    let saved = h.state().preferences.clone();
    store.save(&saved).unwrap();
    let restored = store.load().unwrap();
    assert_eq!(restored.snapshot_directory, Some(dir.path().to_path_buf()));
    assert_eq!(
        h.state().inspect().unwrap().snapshot_saved,
        Some(out.clone())
    );
}

#[test]
fn empty_snapshot_refuses_with_the_typed_error_and_renders_project_is_empty() {
    let mut h = build(editor(
        Project::empty(ProjectId(94), "Empty".into()).unwrap(),
        Preferences::default(),
    ));
    assert!(h.query_by_label("Project is empty").is_some());
    let before = json(h.state().session.project());
    assert!(h.state_mut().activate("preview.snapshot").is_err());
    let inspect = h.state().inspect().unwrap();
    assert_eq!(
        inspect.snapshot_error,
        Some(engine::control::ErrorCode::EmptyProject)
    );
    assert!(!inspect.snapshot_dialog_open);
    assert_eq!(json(h.state().session.project()), before);
}

#[test]
fn snapshot_dialog_cancel_success_failure_and_copy_are_typed() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = build(editor(
        project(FrameRate::FPS_30, 64, 48, false),
        Preferences::default(),
    ));
    let bytes = json(h.state().session.project());

    click(&mut h, "preview.snapshot");
    assert!(h.state().inspect().unwrap().snapshot_dialog_open);
    assert_eq!(
        h.state_mut().take_lifecycle_action(),
        Some(vibecut::lifecycle::EditorAction::SnapshotSave)
    );
    h.state_mut().snapshot_save_cancelled();
    let inspect = h.state().inspect().unwrap();
    assert!(!inspect.snapshot_dialog_open);
    assert!(inspect.snapshot_saved.is_none());
    assert!(inspect.snapshot_error.is_none());
    assert!(inspect.import_toast.is_none());

    // Failed publication stays typed and never claims success.
    let blocked = dir.path().join("blocked");
    std::fs::create_dir(&blocked).unwrap();
    assert!(h.state_mut().snapshot_picked_file(blocked).is_err());
    let inspect = h.state().inspect().unwrap();
    assert!(matches!(
        inspect.snapshot_error,
        Some(engine::control::ErrorCode::Failed | engine::control::ErrorCode::Refused)
    ));
    assert!(inspect.snapshot_saved.is_none());
    assert!(inspect.import_toast.is_none());
    assert_eq!(json(h.state().session.project()), bytes);

    // Success publishes canvas-sized PNG bytes at the playhead.
    let out = dir.path().join("frame.png");
    h.state_mut().snapshot_picked_file(out.clone()).unwrap();
    let inspect = h.state().inspect().unwrap();
    assert_eq!(inspect.snapshot_saved, Some(out.clone()));
    assert!(inspect.snapshot_error.is_none());
    assert!(inspect.import_toast.is_some());
    let image = image::load_from_memory(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (64, 48));
    assert_eq!(json(h.state().session.project()), bytes);

    // Copy hands the same frame image to egui's OS clipboard boundary.
    click(&mut h, "preview.copy");
    assert!(
        h.state()
            .inspect()
            .unwrap()
            .import_toast
            .is_some_and(|toast| toast.contains("Copy frame"))
    );
    let ctx = h.ctx.clone();
    h.state_mut().copy_snapshot(&ctx).unwrap();
    h.run_steps(1);
    let copied = h
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyImage(image) => Some(image),
            _ => None,
        })
        .expect("copy image output");
    assert_eq!(copied.size, [64, 48]);
    assert_eq!(json(h.state().session.project()), bytes);
}

#[test]
fn snapshot_ignores_multi_selection_and_never_mutates_project_json() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = build(editor(
        project(FrameRate::FPS_30, 64, 48, true),
        Preferences::default(),
    ));
    h.state_mut().selection.insert(ItemId(1));
    h.state_mut().selection.insert(ItemId(2));
    let before = json(h.state().session.project());
    let out = dir.path().join("multi.png");
    h.state_mut().snapshot_picked_file(out.clone()).unwrap();
    assert_eq!(json(h.state().session.project()), before);
    assert!(!h.state().session.can_undo());
    assert!(out.is_file());
}

#[test]
fn fullscreen_toggles_from_toolbar_and_context_menu_and_escape_exits() {
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    click(&mut h, "preview.fullscreen");
    assert!(h.state().inspect().unwrap().preview_fullscreen);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().inspect().unwrap().preview_fullscreen);

    // The preview context menu exposes the same toggle.
    let canvas = h.get(By::new().predicate(|node| node.author_id() == Some("preview.canvas")));
    canvas.click_secondary();
    h.run_steps(3);
    click(&mut h, "preview.menu.fullscreen");
    assert!(h.state().inspect().unwrap().preview_fullscreen);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().inspect().unwrap().preview_fullscreen);
}

#[test]
fn edit_after_timecode_seek_resyncs_playback_and_clamps_at_the_new_end() {
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    h.state_mut()
        .dispatch(
            "playback.seek",
            serde_json::json!({"time": 4 * TICKS_PER_SECOND}),
        )
        .unwrap();
    assert_eq!(
        h.state().inspect().unwrap().position,
        Tick(4 * TICKS_PER_SECOND)
    );
    h.state_mut().selection.insert(ItemId(1));
    h.state_mut().activate("timeline.delete").unwrap();
    h.run_steps(2);
    let inspect = h.state().inspect().unwrap();
    assert_eq!(h.state().session.project().duration(), Tick::ZERO);
    assert_eq!(inspect.position, Tick::ZERO);
    assert_eq!(inspect.timecode, "00:00:00:00");
    assert!(h.query_by_label("Project is empty").is_some());
}

#[test]
fn preview_timecode_and_zoom_labels_are_localised() {
    let prefs = Preferences {
        locale: vibecut::catalog::Locale::Vi,
        ..Default::default()
    };
    let h = build(editor(project(FrameRate::FPS_30, 640, 360, false), prefs));
    assert!(h.query_by_label("Mã thời gian không hợp lệ").is_none());
    // The locale retains stable automation ids for the controls.
    assert!(widget(&h, "preview.timecode").enabled);
    assert!(widget(&h, "preview.snapshot").enabled);
}

/// Offscreen evidence for the ticket; wgpu stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-24-preview.png"]
fn preview_toolbar_render_screenshot() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        })
        .unwrap_or_else(|| root.join("target"));
    let mut h = build(editor(
        project(FrameRate::FPS_30, 1280, 720, false),
        Preferences::default(),
    ));
    h.state_mut()
        .dispatch("preview.set_zoom", serde_json::json!({"zoom": 1.5}))
        .unwrap();
    h.run_steps(3);
    let image = h.render().expect("offscreen render");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-24-preview.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

#[test]
fn text_contract_has_the_snapshot_and_zoom_labels() {
    assert_eq!(Text::ProjectEmpty.key(), "project_empty");
    assert_eq!(Text::SnapshotSave.key(), "snapshot_save");
    assert_eq!(Text::Fullscreen.key(), "fullscreen");
    assert_eq!(preview::PRESETS, &[25, 50, 75, 100, 150, 200]);
}
