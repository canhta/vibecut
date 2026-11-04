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
            track,
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
            color: Color([0, 193, 205, 255]),
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
        .tracks
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
        .tracks
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
        .tracks
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
        .tracks
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
