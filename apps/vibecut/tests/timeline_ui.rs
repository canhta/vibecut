#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Command, CreatedId, NewItem, Session};
use media::{
    AssetData, AssetId, AudioInfo, ColorInfo, ContentHash, SourceAsset, SourceProbe, SourceStream,
    TimeBase, VideoInfo,
};
use project::{Color, ItemId, ItemParams, Project, ProjectId, TrackId, TrackKind};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor,
    catalog::{Locale, Preferences},
    egui,
    playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;

struct Fixture {
    editor: Editor,
    video_a: ItemId,
    video_b: ItemId,
}

fn source_stream(kind: media::StreamKind, duration: Tick) -> SourceStream {
    SourceStream {
        index: 0,
        kind,
        codec: match kind {
            media::StreamKind::Video => "h264".into(),
            media::StreamKind::Audio => "aac".into(),
            media::StreamKind::Other => "data".into(),
        },
        time_base: TimeBase::new(1, 1_000).unwrap(),
        duration: Some(duration),
        start: None,
        frame_rate: (kind == media::StreamKind::Video).then_some(FrameRate::FPS_30),
        frame_count: None,
        variable_frame_rate: None,
        video: (kind == media::StreamKind::Video).then_some(VideoInfo {
            width: 1920,
            height: 1080,
            color: ColorInfo::default(),
            bit_depth: Some(8),
        }),
        audio: (kind == media::StreamKind::Audio).then_some(AudioInfo {
            sample_rate: 48_000,
            channels: 2,
            initial_padding: 0,
            trailing_padding: 0,
        }),
    }
}

fn asset(id: u64, kind: media::StreamKind) -> SourceAsset {
    SourceAsset::try_from(AssetData {
        id: AssetId(id),
        path: PathBuf::from(format!("/tmp/vibecut-timeline-{id}.mp4")),
        content_hash: ContentHash::from_bytes(&[id as u8; 8]),
        probe: SourceProbe {
            format: "mov,mp4".into(),
            duration: Some(Tick(10 * SECOND)),
            streams: vec![source_stream(kind, Tick(10 * SECOND))],
        },
    })
    .unwrap()
}

fn insert(
    session: &mut Session,
    track: TrackId,
    start: Tick,
    source: SourceRange,
    asset: Option<AssetId>,
    content: ItemParams,
) -> ItemId {
    let outcome = session
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track },
            item: NewItem {
                animations: Default::default(),
                source_asset: asset,
                source_range: source,
                start,
                content,
            },
        })
        .unwrap();
    match outcome.created.first().copied() {
        Some(CreatedId::Item(id)) => id,
        _ => panic!("item"),
    }
}

fn track(session: &mut Session, name: &str, kind: TrackKind) -> TrackId {
    let outcome = session
        .apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: name.into(),
            kind,
        })
        .unwrap();
    match outcome.created.first().copied() {
        Some(CreatedId::Track(id)) => id,
        _ => panic!("track"),
    }
}

fn fixture() -> Fixture {
    let mut session = Session::new(
        Project::empty(ProjectId(12), "Timeline".into()).unwrap(),
        50,
    )
    .unwrap();
    for (id, kind) in [(1, media::StreamKind::Video), (2, media::StreamKind::Audio)] {
        session
            .apply(Command::AddAsset {
                asset: asset(id, kind),
            })
            .unwrap();
    }
    let video = track(&mut session, "Video", TrackKind::Video);
    let audio = track(&mut session, "Audio", TrackKind::Audio);
    let text = track(&mut session, "Text", TrackKind::Text);
    let video_a = insert(
        &mut session,
        video,
        Tick::ZERO,
        SourceRange::new(Tick::ZERO, Tick(6 * SECOND)).unwrap(),
        Some(AssetId(1)),
        ItemParams::Video(project::VideoParams {
            source_audio_enabled: true,
        }),
    );
    let video_b = insert(
        &mut session,
        video,
        Tick(6 * SECOND),
        SourceRange::new(Tick(6 * SECOND), Tick(10 * SECOND)).unwrap(),
        Some(AssetId(1)),
        ItemParams::Video(project::VideoParams {
            source_audio_enabled: true,
        }),
    );
    let _audio_item = insert(
        &mut session,
        audio,
        Tick::ZERO,
        SourceRange::new(Tick::ZERO, Tick(10 * SECOND)).unwrap(),
        Some(AssetId(2)),
        ItemParams::Audio(project::AudioParams {
            gain_db: 0.0,
            muted: false,
        }),
    );
    let _text_item = insert(
        &mut session,
        text,
        Tick(2 * SECOND),
        SourceRange::new(Tick(2 * SECOND), Tick(5 * SECOND)).unwrap(),
        None,
        ItemParams::Text(project::TextParams {
            content: "Hello".into(),
            font_size: 48.0,
            color: Color([255, 255, 255, 255]),
        }),
    );
    session
        .apply(Command::AddBookmark {
            start: Tick(8 * SECOND),
            duration: Tick::ZERO,
            note: "Beat".into(),
            color: Color([0xD1, 0xFE, 0x17, 255]),
        })
        .unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Fixture {
        editor: Editor::new(session, playback, Preferences::default()),
        video_a,
        video_b,
    }
}

fn harness(editor: Editor) -> Harness<'static, Editor> {
    Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn widget(harness: &Harness<'_, Editor>, id: &str) -> egui::Rect {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == id)
        .unwrap_or_else(|| panic!("missing widget {id}"))
        .rect
}

fn click_at(harness: &mut Harness<'_, Editor>, pos: egui::Pos2) {
    harness.drag_at(pos);
    harness.run_steps(1);
    harness.drop_at(pos);
    harness.run_steps(2);
}

fn drag(harness: &mut Harness<'_, Editor>, from: egui::Pos2, to: egui::Pos2) {
    harness.drag_at(from);
    harness.run_steps(1);
    harness.hover_at(to);
    harness.run_steps(2);
    harness.drop_at(to);
    harness.run_steps(2);
}

fn item(editor: &Editor, id: ItemId) -> &project::Item {
    editor
        .session
        .project()
        .data()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == id)
        .unwrap()
}

fn count(editor: &Editor) -> usize {
    editor
        .session
        .project()
        .data()
        .timeline
        .iter()
        .map(|track| track.items.len())
        .sum()
}

#[test]
fn ruler_click_split_and_undo_redo_use_automation_ids_and_engine_commands() {
    let fixture = fixture();
    let video_a = fixture.video_a;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let inspection = harness.state().inspect().unwrap();
    for id in [
        "timeline.ruler".to_string(),
        "timeline.split".to_string(),
        "timeline.playhead".to_string(),
        "timeline.snap".to_string(),
        "timeline.scroll_h".to_string(),
        "timeline.scroll_v".to_string(),
        format!("timeline.item.{}", video_a.0),
        format!("timeline.item.{}.trim_start", video_a.0),
        format!("timeline.item.{}.trim_end", video_a.0),
    ] {
        assert!(
            inspection.widgets.iter().any(|widget| widget.id == id),
            "missing {id}"
        );
    }
    let ruler = widget(&harness, "timeline.ruler");
    let zoom = inspection.zoom;
    click_at(
        &mut harness,
        egui::pos2(ruler.left() + 3.0 * zoom, ruler.center().y),
    );
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(3 * SECOND)
    );
    let item_rect = widget(&harness, &format!("timeline.item.{}", video_a.0));
    click_at(
        &mut harness,
        egui::pos2(item_rect.left() + 20.0, item_rect.center().y),
    );
    assert_eq!(harness.state().inspect().unwrap().selection, vec![video_a]);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.run_steps(2);
    assert_eq!(count(harness.state()), 5, "split adds one item");
    let selected = harness.state().inspect().unwrap().selection;
    assert_eq!(selected.len(), 1);
    assert_eq!(item(harness.state(), selected[0]).start, Tick(3 * SECOND));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(count(harness.state()), 4, "undo restores one item");
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::Z,
    );
    harness.run_steps(2);
    assert_eq!(count(harness.state()), 5, "redo splits again");
    let right = harness
        .state()
        .session
        .project()
        .data()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.start == Tick(3 * SECOND))
        .unwrap()
        .id;
    let rect = widget(&harness, &format!("timeline.item.{}", right.0));
    click_at(&mut harness, egui::pos2(rect.center().x, rect.center().y));
    assert_eq!(harness.state().inspect().unwrap().selection, vec![right]);
    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    assert_eq!(
        count(harness.state()),
        4,
        "delete removes the selected half"
    );
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(count(harness.state()), 5, "undo restores the deleted half");
}

#[test]
fn trim_and_move_drag_commit_one_undo_step_each() {
    let fixture = fixture();
    let video_a = fixture.video_a;
    let video_b = fixture.video_b;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let zoom = harness.state().inspect().unwrap().zoom;
    let timeline = harness.state().panels["timeline"];
    let body_left = timeline.left() + 12.0 + 112.0;
    let end_handle = widget(&harness, &format!("timeline.item.{}.trim_end", video_a.0));
    drag(
        &mut harness,
        end_handle.center(),
        egui::pos2(body_left + 5.0 * zoom, end_handle.center().y),
    );
    assert_eq!(
        item(harness.state(), video_a).end().unwrap(),
        Tick(5 * SECOND)
    );
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(
        item(harness.state(), video_a).end().unwrap(),
        Tick(6 * SECOND)
    );

    let body = widget(&harness, &format!("timeline.item.{}", video_b.0));
    drag(
        &mut harness,
        body.center(),
        egui::pos2(body.center().x + 1.0 * zoom, body.center().y),
    );
    assert_eq!(item(harness.state(), video_b).start, Tick(7 * SECOND));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(item(harness.state(), video_b).start, Tick(6 * SECOND));
}

#[test]
fn selection_modifiers_snapping_toggle_and_ripple_delete_reach_engine() {
    let fixture = fixture();
    let video_a = fixture.video_a;
    let video_b = fixture.video_b;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    harness
        .get(By::new().predicate(|node| {
            node.author_id() == Some(format!("timeline.item.{}", video_a.0).as_str())
        }))
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|node| {
            node.author_id() == Some(format!("timeline.item.{}", video_b.0).as_str())
        }))
        .click_modifiers(egui::Modifiers::COMMAND);
    harness.run_steps(2);
    let inspection = harness.state().inspect().unwrap();
    assert_eq!(inspection.selection.len(), 2);
    assert!(inspection.snapping);
    harness
        .get(By::new().predicate(|node| {
            node.author_id() == Some(format!("timeline.item.{}", video_b.0).as_str())
        }))
        .click_modifiers(egui::Modifiers::COMMAND);
    harness.run_steps(2);
    assert_eq!(harness.state().inspect().unwrap().selection, vec![video_a]);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.snap")))
        .click();
    harness.run_steps(2);
    assert!(!harness.state().inspect().unwrap().snapping);
    harness.key_press(egui::Key::N);
    harness.run_steps(2);
    assert!(harness.state().inspect().unwrap().snapping);
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Delete);
    harness.run_steps(2);
    assert!(item_opt(harness.state(), video_a).is_none());
    assert_eq!(
        item(harness.state(), video_b).start,
        Tick::ZERO,
        "ripple delete closes the vacated tail on the same track"
    );
}

#[test]
fn snapping_moves_to_a_bookmark_and_shift_bypasses_it() {
    let fixture = fixture();
    let video_b = fixture.video_b;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let zoom = harness.state().inspect().unwrap().zoom;
    let body = widget(&harness, &format!("timeline.item.{}", video_b.0));
    drag(
        &mut harness,
        body.center(),
        egui::pos2(body.center().x + 2.1 * zoom, body.center().y),
    );
    assert_eq!(
        item(harness.state(), video_b).start,
        Tick(8 * SECOND),
        "an 8 s bookmark pulls the 8.1 s item start"
    );
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(item(harness.state(), video_b).start, Tick(6 * SECOND));
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    let body = widget(&harness, &format!("timeline.item.{}", video_b.0));
    drag(
        &mut harness,
        body.center(),
        egui::pos2(body.center().x + 2.1 * zoom, body.center().y),
    );
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    let start = item(harness.state(), video_b).start.seconds();
    assert!(
        (start - 8.1).abs() < 0.01,
        "Shift bypasses snapping, got {start}"
    );
}

fn item_opt(editor: &Editor, id: ItemId) -> Option<&project::Item> {
    editor
        .session
        .project()
        .data()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == id)
}

fn screenshot_dir() -> PathBuf {
    if let Some(target) = std::env::var_os("CARGO_TARGET_DIR") {
        PathBuf::from(target).join("screenshots")
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots")
    }
}

fn render_receipt(
    editor: Editor,
    name: &str,
    dark: bool,
    locale: Locale,
    select: ItemId,
) -> (PathBuf, PathBuf) {
    let mut editor = editor;
    editor.preferences.dark = dark;
    editor.preferences.locale = locale;
    let project = editor.session.project().clone();
    editor
        .playback
        .seek(Tick(10 * SECOND), false, project)
        .unwrap();
    let mut harness = harness(editor);
    harness.run_steps(4);
    harness
        .get(By::new().predicate(|node| {
            node.author_id() == Some(format!("timeline.item.{}", select.0).as_str())
        }))
        .click();
    harness.run_steps(2);
    let inspection = harness.state().inspect().unwrap();
    let image = harness.render().unwrap();
    assert_eq!(image.width(), 1440);
    assert_eq!(image.height(), 900);
    let mut colors = BTreeSet::new();
    for pixel in image.pixels().step_by(97) {
        colors.insert(pixel.0);
    }
    assert!(
        colors.len() > 32,
        "{name} screenshot must contain the drawn editor, found {} colours",
        colors.len()
    );
    let widgets: Vec<serde_json::Value> = inspection
        .widgets
        .iter()
        .map(|widget| {
            serde_json::json!({
                "id": widget.id,
                "enabled": widget.enabled,
                "rect": [widget.rect.min.x, widget.rect.min.y, widget.rect.max.x, widget.rect.max.y],
            })
        })
        .collect();
    let receipt = serde_json::json!({
        "name": name,
        "theme": if dark { "dark" } else { "light" },
        "locale": if locale == Locale::En { "en" } else { "vi" },
        "playhead_ticks": inspection.position.0,
        "zoom_px_per_second": inspection.zoom,
        "snapping": inspection.snapping,
        "selection": inspection.selection.iter().map(|id| id.0).collect::<Vec<_>>(),
        "widgets": widgets,
    });
    let dir = screenshot_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let image_path = dir.join(format!("{name}.png"));
    let receipt_path = dir.join(format!("{name}.json"));
    image.save(&image_path).unwrap();
    std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    (image_path, receipt_path)
}

#[test]
fn timeline_screenshot_proof_after_automation_id_edits() {
    let dark_fixture = fixture();
    let video = dark_fixture.video_a;
    let (dark_path, dark_receipt) = render_receipt(
        dark_fixture.editor,
        "timeline-dark",
        true,
        Locale::En,
        video,
    );
    assert!(dark_path.is_file());
    assert!(dark_receipt.is_file());
    let light_fixture = fixture();
    let (light_path, light_receipt) = render_receipt(
        light_fixture.editor,
        "timeline-light-vi",
        false,
        Locale::Vi,
        light_fixture.video_a,
    );
    assert!(light_path.is_file());
    assert!(light_receipt.is_file());
    println!(
        "timeline screenshots: {} {}",
        dark_path.display(),
        light_path.display()
    );
}

#[test]
fn every_editor_button_reports_its_embedded_icon() {
    let mut harness = harness(fixture().editor);
    harness.run_steps(2);
    let widgets = harness.state().inspect().unwrap().widgets;
    let icon = |id: &str| {
        widgets
            .iter()
            .find(|widget| widget.id == id)
            .unwrap_or_else(|| panic!("missing widget {id}"))
            .icon
    };
    let tooltip = |id: &str| {
        widgets
            .iter()
            .find(|widget| widget.id == id)
            .unwrap_or_else(|| panic!("missing widget {id}"))
            .tooltip
            .as_deref()
    };
    let expected = [
        ("project.back", "arrow-left"),
        ("project.rename", "pencil-simple"),
        ("file.export", "export"),
        ("project.save_as", "floppy-disk-back"),
        ("project.save", "floppy-disk"),
        ("appearance.theme", "circle-half"),
        ("appearance.locale", "translate"),
        ("layout.reset", "corners-out"),
        ("file.import", "upload-simple"),
        ("media.insert", "plus"),
        ("media.asset.1", "film-strip"),
        ("media.asset.2", "music-notes"),
        ("transport.back", "skip-back"),
        ("transport.play", "play"),
        ("transport.forward", "skip-forward"),
        ("preview.retry", "arrow-clockwise"),
        ("edit.undo", "arrow-counter-clockwise"),
        ("edit.redo", "arrow-clockwise"),
        ("timeline.split", "scissors"),
        ("timeline.delete", "trash"),
        ("timeline.snap", "arrows-in-line-horizontal"),
        ("timeline.magnet", "magnet"),
        ("timeline.ripple", "waveform"),
        ("timeline.zoom_out", "magnifying-glass-minus"),
        ("timeline.zoom_in", "magnifying-glass-plus"),
        ("track.1.header", "film-strip"),
        ("track.3.header", "text-t"),
    ];
    for (id, name) in expected {
        assert_eq!(icon(id), Some(name), "{id} must carry the {name} icon");
    }
    let redo = widgets
        .iter()
        .find(|widget| widget.id == "edit.redo")
        .unwrap();
    assert!(
        !redo.enabled,
        "unavailable redo must report disabled to agents"
    );
    assert_eq!(icon("inspector.project"), None);
    assert_eq!(icon("inspector.details"), None);
    let split = widgets
        .iter()
        .find(|widget| widget.id == "timeline.split")
        .unwrap();
    assert!(
        (split.rect.height() - 28.0).abs() < 0.5 && (split.rect.width() - 28.0).abs() < 0.5,
        "timeline actions keep the 28x28 icon-only hit box, got {:?}",
        split.rect
    );
    let timeline = harness.state().panels["timeline"];
    assert!(
        (split.rect.min.y - timeline.top() - 18.0).abs() < 0.5,
        "timeline toolbar keeps its 40 px row"
    );
    for (id, text) in [
        ("edit.undo", "Undo (Cmd/Ctrl+Z)"),
        ("timeline.split", "Split (Cmd/Ctrl+B)"),
        ("timeline.snap", "Snap (N)"),
        ("timeline.zoom_in", "Zoom +"),
        ("timeline.magnet", "Main track magnet"),
        ("transport.back", "Back 1 second (J)"),
        ("transport.play", "Play (Space)"),
        ("preview.retry", "Retry preview"),
    ] {
        assert_eq!(tooltip(id), Some(text), "{id} tooltip");
    }
}

fn text_item_id(editor: &Editor) -> ItemId {
    editor
        .session
        .project()
        .data()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| matches!(item.content, ItemParams::Text(_)))
        .unwrap()
        .id
}

#[test]
fn review_delete_invalidates_preview_and_clamps_playhead() {
    let mut session = Session::new(
        Project::empty(ProjectId(123), "Preview".into()).unwrap(),
        20,
    )
    .unwrap();
    let overlay = track(&mut session, "Overlay", TrackKind::Video);
    let item_id = insert(
        &mut session,
        overlay,
        Tick::ZERO,
        SourceRange::new(Tick::ZERO, Tick(10 * SECOND)).unwrap(),
        None,
        ItemParams::Overlay(project::OverlayParams {
            color: Color([255, 0, 0, 255]),
        }),
    );
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    playback
        .seek(Tick(5 * SECOND), false, session.project().clone())
        .unwrap();
    let mut editor = Editor::new(session, playback, Preferences::default());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while editor.playback.frame().is_none() {
        editor.tick();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    editor.selection.insert(item_id);
    editor.activate("timeline.delete").unwrap();
    assert_eq!(editor.session.project().duration(), Tick::ZERO);
    assert_eq!(
        editor.playback.position().unwrap(),
        Tick::ZERO,
        "delete must clamp the old 5 s playhead to the new duration"
    );
    assert!(
        editor.playback.frame().is_none(),
        "delete must invalidate frames from the previous project snapshot"
    );
}

#[test]
fn review_offscreen_drop_closes_gesture() {
    let fixture = fixture();
    let id = fixture.video_b;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let from = widget(&harness, &format!("timeline.item.{}", id.0)).center();
    drag(&mut harness, from, from + egui::vec2(1600.0, 0.0));
    assert!(
        !harness.state().session.gesture_active(),
        "drop must close the gesture even when the item is offscreen"
    );
}

#[test]
fn review_group_move_preserves_spacing_at_zero() {
    let fixture = fixture();
    let text = text_item_id(&fixture.editor);
    let mut editor = fixture.editor;
    editor
        .session
        .apply(Command::Delete {
            items: vec![fixture.video_a],
        })
        .unwrap();
    editor.selection = [fixture.video_b, text].into_iter().collect();
    let id = fixture.video_b;
    let mut harness = harness(editor);
    harness.run_steps(3);
    let from = widget(&harness, &format!("timeline.item.{}", id.0)).center();
    drag(&mut harness, from, from - egui::vec2(160.0, 0.0));
    assert_eq!(
        item(harness.state(), id).start - item(harness.state(), text).start,
        Tick(4 * SECOND),
        "group spacing must survive reaching timeline zero"
    );
}

#[test]
fn review_move_back_to_origin_restores_position() {
    for snapping in [true, false] {
        let fixture = fixture();
        let id = fixture.video_b;
        let mut editor = fixture.editor;
        editor.snapping = snapping;
        let mut harness = harness(editor);
        harness.run_steps(3);
        let body = widget(&harness, &format!("timeline.item.{}", id.0));
        let from = body.center();
        harness.drag_at(from);
        harness.run_steps(1);
        harness.hover_at(from + egui::vec2(40.0, 0.0));
        harness.run_steps(2);
        assert_eq!(item(harness.state(), id).start, Tick(7 * SECOND));
        harness.hover_at(from);
        harness.run_steps(2);
        harness.drop_at(from);
        harness.run_steps(2);
        assert_eq!(
            item(harness.state(), id).start,
            Tick(6 * SECOND),
            "returning to the press point restores the entry position (snapping {snapping})"
        );
    }
}

#[test]
fn review_trim_release_survives_its_handle_becoming_narrow() {
    let fixture = fixture();
    let id = fixture.video_a;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let zoom = harness.state().inspect().unwrap().zoom;
    let timeline = harness.state().panels["timeline"];
    let body_left = timeline.left() + 12.0 + 112.0;
    let handle = widget(&harness, &format!("timeline.item.{}.trim_end", id.0));
    drag(
        &mut harness,
        handle.center(),
        egui::pos2(body_left + 0.05 * zoom, handle.center().y),
    );
    assert!(!harness.state().session.gesture_active());
    let trimmed = item(harness.state(), id);
    assert!(trimmed.end().unwrap() > trimmed.start);
    assert!(
        trimmed.end().unwrap() < Tick(SECOND),
        "trim handle must close its gesture after the clip becomes too narrow to hit-test"
    );
}

#[test]
fn review_split_respects_selection_outside_playhead() {
    let fixture = fixture();
    let text = text_item_id(&fixture.editor);
    let mut editor = fixture.editor;
    editor.selection.insert(text);
    editor
        .playback
        .seek(Tick(7 * SECOND), false, editor.session.project().clone())
        .unwrap();
    let before = editor.session.project().clone();
    editor.activate("timeline.split").unwrap();
    assert_eq!(
        editor.session.project(),
        &before,
        "selected 2-5 s text must prevent splitting unrelated video/audio at 7 s"
    );
}

#[test]
fn edits_while_playing_restart_preview_and_pause_at_the_new_end() {
    let mut session = Session::new(
        Project::empty(ProjectId(124), "Playing".into()).unwrap(),
        20,
    )
    .unwrap();
    let overlay = track(&mut session, "Overlay", TrackKind::Video);
    let item_id = insert(
        &mut session,
        overlay,
        Tick::ZERO,
        SourceRange::new(Tick::ZERO, Tick(10 * SECOND)).unwrap(),
        None,
        ItemParams::Overlay(project::OverlayParams {
            color: Color([255, 0, 0, 255]),
        }),
    );
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    editor.activate("transport.play").unwrap();
    let port = editor.playback.audio_port();
    for _ in 0..25 {
        port.fill(&mut [0.0; 960], 0);
        editor.tick();
    }
    assert!(editor.playback.position().unwrap() > Tick::ZERO);
    editor.selection.insert(item_id);
    editor.activate("timeline.delete").unwrap();
    assert_eq!(editor.session.project().duration(), Tick::ZERO);
    assert!(editor.playback.frame().is_none());
    editor.tick();
    assert!(!editor.playback.playing());
    assert_eq!(editor.playback.position().unwrap(), Tick::ZERO);
}

#[test]
fn escape_rolls_back_the_drag_and_restarts_preview() {
    let fixture = fixture();
    let id = fixture.video_b;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let body = widget(&harness, &format!("timeline.item.{}", id.0));
    let from = body.center();
    harness.drag_at(from);
    harness.run_steps(1);
    harness.hover_at(from + egui::vec2(40.0, 0.0));
    harness.run_steps(2);
    assert_eq!(item(harness.state(), id).start, Tick(7 * SECOND));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(item(harness.state(), id).start, Tick(6 * SECOND));
    assert!(!harness.state().session.gesture_active());
    harness.drop_at(from);
    harness.run_steps(2);
    assert_eq!(item(harness.state(), id).start, Tick(6 * SECOND));
}

#[test]
fn lifecycle_view_state_restores_and_tracks_timeline_navigation() {
    let mut fixture = fixture();
    fixture
        .editor
        .session
        .execute(
            "project.set_view_state",
            serde_json::json!({"zoom":5,"scroll":80,"playhead":0}),
        )
        .unwrap();
    let vibecut::lifecycle::EditorSession::Memory(session) = fixture.editor.session else {
        panic!("memory fixture")
    };
    let editor = Editor::new(
        *session,
        fixture.editor.playback,
        fixture.editor.preferences,
    );
    assert_eq!(editor.inspect().unwrap().zoom, 200.0);
    let mut harness = harness(editor);
    harness.run_steps(3);
    let view = harness.state().session.metadata().view;
    assert_eq!(view.zoom, 5.0);
    assert!((view.scroll - 80.0).abs() < 0.001);
    harness.state_mut().activate("timeline.zoom_in").unwrap();
    harness.run_steps(2);
    let zoom = harness.state().inspect().unwrap().zoom;
    assert!((harness.state().session.metadata().view.zoom - f64::from(zoom) / 40.0).abs() < 1e-6);
    let scrollbar = widget(&harness, "timeline.scroll_h");
    click_at(&mut harness, scrollbar.center());
    assert!(harness.state().session.metadata().view.scroll > 80.0);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(
        harness.state().inspect().unwrap().zoom,
        zoom,
        "navigation stays outside edit history"
    );
}

#[test]
fn rename_field_suppresses_timeline_edit_shortcuts() {
    let mut fixture = fixture();
    fixture.editor.selection.insert(fixture.video_a);
    fixture
        .editor
        .playback
        .seek(
            Tick(3 * SECOND),
            false,
            fixture.editor.session.project().clone(),
        )
        .unwrap();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    harness.state_mut().activate("project.rename").unwrap();
    harness.run_steps(2);
    let before = harness.state().session.project().clone();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    assert_eq!(harness.state().session.project(), &before);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().session.project(), &before);
}

#[test]
fn engine_view_command_is_not_overwritten_by_the_next_editor_action() {
    let mut fixture = fixture();
    fixture
        .editor
        .session
        .execute(
            "project.set_view_state",
            serde_json::json!({"zoom":2,"scroll":40,"playhead":3 * SECOND}),
        )
        .unwrap();
    fixture.editor.activate("transport.pause").unwrap();
    assert_eq!(fixture.editor.inspect().unwrap().zoom, 80.0);
    assert_eq!(
        fixture.editor.playback.position().unwrap(),
        Tick(3 * SECOND)
    );
    assert_eq!(fixture.editor.session.metadata().view.zoom, 2.0);
    assert!((fixture.editor.session.metadata().view.scroll - 40.0).abs() < 0.001);
    fixture
        .editor
        .session
        .execute(
            "project.set_view_state",
            serde_json::json!({"zoom":3,"scroll":0,"playhead":0}),
        )
        .unwrap();
    fixture.editor.activate("timeline.zoom_in").unwrap();
    assert!(
        (fixture.editor.inspect().unwrap().zoom - 204.0).abs() < 0.001,
        "a relative UI action starts from the latest engine-issued view"
    );
}

#[test]
fn marquee_reversal_and_escape_scopes_leave_project_unchanged() {
    use vibecut::selection::SelectionScope;
    let fixture = fixture();
    let a = fixture.video_a;
    let b = fixture.video_b;
    let bytes = serde_json::to_vec(fixture.editor.session.project().data()).unwrap();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let ar = widget(&harness, &format!("timeline.item.{}", a.0));
    let br = widget(&harness, &format!("timeline.item.{}", b.0));
    let body = widget(&harness, "timeline.marquee");
    // The test derives geometry from real automation IDs; no OS/window coordinates.
    let origin = egui::pos2(br.right() + 12.0, ar.bottom() + 3.0);
    assert!(body.contains(origin));
    harness.drag_at(origin);
    harness.run_steps(1);
    harness.hover_at(ar.left_top() - egui::vec2(1.0, 1.0));
    harness.run_steps(2);
    assert_eq!(harness.state().selection, [a, b].into_iter().collect());
    harness.hover_at(egui::pos2(br.left() + 5.0, ar.top() - 1.0));
    harness.run_steps(2);
    assert_eq!(harness.state().selection, [b].into_iter().collect());
    harness.drop_at(ar.left_top() - egui::vec2(1.0, 1.0));
    harness.run_steps(2);
    assert_eq!(harness.state().selection, [a, b].into_iter().collect());
    harness.drag_at(origin);
    harness.run_steps(1);
    harness.hover_at(egui::pos2(br.left() + 5.0, ar.top() - 1.0));
    harness.run_steps(2);
    assert_eq!(harness.state().selection, [b].into_iter().collect());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().selection, [a, b].into_iter().collect());
    harness.drop_at(origin);
    harness.run_steps(2);
    harness.state_mut().selection_scopes.mask_points.insert(
        vibecut::selection::MaskPointSelection {
            item: a,
            mask: 0,
            point: 0,
        },
    );
    harness
        .state_mut()
        .selection_scopes
        .keyframes
        .insert(vibecut::selection::KeyframeSelection {
            item: a,
            path: project::AnimationPath::new("opacity").unwrap(),
            component: None,
            time: Tick::ZERO,
        });
    for scope in [
        SelectionScope::MaskPoints,
        SelectionScope::Keyframes,
        SelectionScope::Items,
    ] {
        assert_eq!(harness.state().inspect().unwrap().selection_scope, scope);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
    }
    assert!(harness.state().selection.is_empty());
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(2);
    assert_eq!(harness.state().selection.len(), count(harness.state()));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(harness.state().selection.is_empty());
    assert_eq!(
        serde_json::to_vec(harness.state().session.project().data()).unwrap(),
        bytes
    );
}

fn two_text_fixture() -> (Fixture, ItemId, ItemId) {
    let mut fixture = fixture();
    let text = fixture
        .editor
        .session
        .project()
        .data()
        .timeline
        .iter()
        .find(|t| t.kind == TrackKind::Text)
        .unwrap()
        .id;
    let first = fixture
        .editor
        .session
        .project()
        .data()
        .timeline
        .iter()
        .find(|t| t.id == text)
        .unwrap()
        .items[0]
        .id;
    let second = insert(
        &mut fixture.editor.session,
        text,
        Tick(6 * SECOND),
        SourceRange::new(Tick::ZERO, Tick(2 * SECOND)).unwrap(),
        None,
        ItemParams::Text(project::TextParams {
            content: "Khác".into(),
            font_size: 24.0,
            color: Color([20, 30, 40, 255]),
        }),
    );
    (fixture, first, second)
}

#[test]
fn mixed_text_inspector_applies_one_field_and_restores_selected_items() {
    let (fixture, first, second) = two_text_fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    for (id, modifiers) in [
        (first, egui::Modifiers::NONE),
        (second, egui::Modifiers::SHIFT),
    ] {
        harness
            .get(
                By::new().predicate(|n| {
                    n.author_id() == Some(format!("timeline.item.{}", id.0).as_str())
                }),
            )
            .click_modifiers(modifiers);
        harness.run_steps(2);
    }
    let fields = vibecut::selection::shared_fields(
        harness.state().session.project(),
        &harness.state().selection,
    );
    assert_eq!(fields.len(), 2);
    assert!(
        fields
            .iter()
            .all(|f| f.value.is_none() && f.compatible == 2)
    );
    let before = harness.state().session.project().clone();
    let history = harness.state().session.history_len().0;
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.text_size")))
        .click();
    harness.run_steps(1);
    harness.event(egui::Event::Text("64".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(harness.state().session.history_len().0, history + 1);
    for id in [first, second] {
        let ItemParams::Text(p) = &item(harness.state(), id).content else {
            panic!("text")
        };
        assert_eq!(p.font_size, 64.0);
    }
    harness
        .state_mut()
        .activate("timeline.deselect_all")
        .unwrap();
    harness.state_mut().activate("edit.undo").unwrap();
    assert_eq!(harness.state().session.project(), &before);
    assert_eq!(
        harness.state().selection,
        [first, second].into_iter().collect()
    );
    harness.state_mut().activate("edit.redo").unwrap();
    assert_eq!(
        harness.state().selection,
        [first, second].into_iter().collect()
    );
    for invalid in ["NaN", "0", "4097", "-1"] {
        assert!(
            harness
                .state_mut()
                .set_shared_field("text_size", invalid)
                .is_err()
        );
    }
    assert!(
        harness
            .state_mut()
            .set_shared_field("text_color", "💥")
            .is_err()
    );
    harness
        .state_mut()
        .set_shared_field("text_color", "#11223344")
        .unwrap();
    for id in [first, second] {
        let ItemParams::Text(p) = &item(harness.state(), id).content else {
            panic!("text")
        };
        assert_eq!(p.font_size, 64.0);
        assert_eq!(p.color, Color([17, 34, 51, 68]));
    }
}

#[test]
#[ignore = "offscreen visual audit, run explicitly outside CI"]
fn selection_inspector_visual_audit() {
    for (scale, dark, locale, filename) in [
        (1.0, true, Locale::En, "p1-19-selection.png"),
        (2.0, false, Locale::Vi, "p1-19-selection-light-vi-2x.png"),
    ] {
        let (mut fixture, first, second) = two_text_fixture();
        fixture.editor.preferences.dark = dark;
        fixture.editor.preferences.locale = locale;
        let media: Vec<_> = fixture
            .editor
            .session
            .project()
            .data()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .filter(|i| matches!(i.content, ItemParams::Video(_) | ItemParams::Audio(_)))
            .map(|i| i.id)
            .collect();
        fixture
            .editor
            .session
            .apply(Command::Delete { items: media })
            .unwrap();
        let main = fixture.editor.session.project().data().timeline.main.id;
        let background = insert(
            &mut fixture.editor.session,
            main,
            Tick::ZERO,
            SourceRange::new(Tick::ZERO, Tick(10 * SECOND)).unwrap(),
            None,
            ItemParams::Overlay(project::OverlayParams {
                color: Color([35, 45, 65, 255]),
            }),
        );
        fixture.editor.selection = [first, second, background].into_iter().collect();
        let project = fixture.editor.session.project().clone();
        fixture
            .editor
            .playback
            .seek(Tick(3 * SECOND), false, project)
            .unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_pixels_per_point(scale)
            .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
        harness.run_steps(4);
        let image = harness.render().unwrap();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(filename);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image.save(&path).unwrap();
        println!("selection audit: {}", path.display());
    }
}

#[test]
fn compatible_audio_gain_inspector_skips_video_and_preserves_mute() {
    let fixture = fixture();
    let video = fixture.video_a;
    let audio = fixture
        .editor
        .session
        .project()
        .data()
        .timeline
        .iter()
        .find(|t| t.kind == TrackKind::Audio)
        .unwrap()
        .items[0]
        .id;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    for (id, modifiers) in [
        (video, egui::Modifiers::NONE),
        (audio, egui::Modifiers::CTRL),
    ] {
        harness
            .get(
                By::new().predicate(|n| {
                    n.author_id() == Some(format!("timeline.item.{}", id.0).as_str())
                }),
            )
            .click_modifiers(modifiers);
        harness.run_steps(2);
    }
    let before = item(harness.state(), video).clone();
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.audio_gain")))
        .click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(1);
    harness.event(egui::Event::Text("-6".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(
        harness.state().selection,
        [video, audio].into_iter().collect()
    );
    assert_eq!(item(harness.state(), video), &before);
    let ItemParams::Audio(p) = &item(harness.state(), audio).content else {
        panic!("audio")
    };
    assert_eq!(p.gain_db, -6.0);
    assert!(!p.muted);
}

#[test]
fn drop_at_track_edges_creates_one_track_and_one_undo_step() {
    for below in [false, true] {
        let fixture = fixture();
        let moving = fixture.video_a;
        let mut h = Harness::builder()
            .with_size(egui::vec2(1728.0, 1200.0))
            .build_ui_state(|ui, e: &mut Editor| e.ui(ui), fixture.editor);
        h.run_steps(3);
        let before = h.state().session.project().clone();
        let history = h.state().session.history_len();
        let source = before
            .data()
            .timeline
            .overlays
            .iter()
            .find(|t| t.items.iter().any(|i| i.id == moving))
            .unwrap()
            .id;
        let index = before
            .data()
            .timeline
            .overlays
            .iter()
            .position(|t| t.id == source)
            .unwrap()
            + usize::from(below);
        let body = widget(&h, &format!("timeline.item.{}", moving.0));
        let header = widget(&h, &format!("track.{}.header", source.0));
        let target = egui::pos2(
            body.center().x,
            if below {
                header.bottom() - 1.0
            } else {
                header.top() + 1.0
            },
        );
        drag(&mut h, body.center(), target);
        let after = h.state().session.project().clone();
        assert_eq!(
            after.data().timeline.track_count(),
            before.data().timeline.track_count() + 1
        );
        let new = &after.data().timeline.overlays[index];
        assert_eq!(new.items.len(), 1);
        assert_eq!(new.items[0].id, moving);
        assert_eq!(new.items[0].start, Tick::ZERO);
        assert_eq!(h.state().session.history_len().0, history.0 + 1);
        if below {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/p1-14-placement.png");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            h.render().unwrap().save(&path).unwrap();
            println!("placement screenshot: {}", path.display());
        }
        h.state_mut().session.undo().unwrap();
        assert_eq!(h.state().session.project(), &before);
        h.state_mut().session.redo().unwrap();
        assert_eq!(h.state().session.project(), &after);
        // A video cannot create an overlay inside the audio group; roll back the whole gesture.
        h.run_steps(3);
        let audio = after.data().timeline.audio[0].id;
        let audio_header = widget(&h, &format!("track.{}.header", audio.0));
        let body = widget(&h, &format!("timeline.item.{}", moving.0));
        let history = h.state().session.history_len();
        drag(
            &mut h,
            body.center(),
            egui::pos2(body.center().x + 20.0, audio_header.bottom() - 1.0),
        );
        assert_eq!(h.state().session.project(), &after);
        assert_eq!(h.state().session.history_len(), history);
    }
}
