#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use engine::{ByteBudget, Command, CreatedId, NewItem, Session};
use media::{AssetData, AssetId, ContentHash, SourceAsset, SourceProbe, SourceStream};
use project::{Color, ItemId, ItemParams, Project, ProjectId, TrackId, TrackKind};
use std::{path::PathBuf, sync::Arc};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    AppCommand, Editor, egui,
    i18n::Locale,
    preferences::Preferences,
    services::playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;

pub(crate) struct Fixture {
    pub(crate) editor: Editor,
    pub(crate) video_a: ItemId,
    pub(crate) video_b: ItemId,
}

fn source_stream(kind: media::StreamKind, duration: Tick) -> SourceStream {
    testkit::source::stream(kind).duration(duration).build()
}

fn asset(id: u64, kind: media::StreamKind) -> SourceAsset {
    testkit::source::asset(
        id,
        std::env::temp_dir().join(format!("vibecut-timeline-{id}.mp4")),
    )
    .format("mov,mp4")
    .duration(Tick(10 * SECOND))
    .content_hash(ContentHash::from_bytes(&[id as u8; 8]))
    .stream(source_stream(kind, Tick(10 * SECOND)))
    .build()
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
                retime: Default::default(),
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

pub(crate) fn fixture() -> Fixture {
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
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
        }),
    );
    let video_b = insert(
        &mut session,
        video,
        Tick(6 * SECOND),
        SourceRange::new(Tick(6 * SECOND), Tick(10 * SECOND)).unwrap(),
        Some(AssetId(1)),
        ItemParams::Video(project::VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
        }),
    );
    let _audio_item = insert(
        &mut session,
        audio,
        Tick::ZERO,
        SourceRange::new(Tick::ZERO, Tick(10 * SECOND)).unwrap(),
        Some(AssetId(2)),
        ItemParams::Audio(project::AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
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
            ..Default::default()
        }),
    );
    session
        .apply(Command::BookmarkAdd {
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

pub(crate) fn harness(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

/// An editor over `project` with the given presentation preferences.
fn editor_for(project: Project, preferences: Preferences) -> Editor {
    let session = Session::new(project, 50).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Editor::new(session, playback, preferences)
}

#[test]
fn ruler_bookmark_menu_jumps_and_disables_at_boundaries() {
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
    let mut harness = harness(editor);
    harness.run_steps(3);
    let ruler = harness.get(By::new().predicate(|node| node.author_id() == Some("timeline.ruler")));
    ruler.click_secondary();
    harness.run_steps(3);
    let inspected = harness.state().inspect().unwrap();
    assert!(
        inspected
            .widgets
            .iter()
            .find(|widget| widget.id == "timeline.menu.jump.next")
            .unwrap()
            .enabled
    );
    assert!(
        !inspected
            .widgets
            .iter()
            .find(|widget| widget.id == "timeline.menu.jump.previous")
            .unwrap()
            .enabled
    );
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.menu.jump.next")))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(3 * SECOND)
    );

    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.ruler")))
        .click_secondary();
    harness.run_steps(3);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.menu.jump.next")))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(8 * SECOND)
    );
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.ruler")))
        .click_secondary();
    harness.run_steps(3);
    assert!(
        !harness
            .state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .find(|widget| widget.id == "timeline.menu.jump.next")
            .unwrap()
            .enabled
    );
}

#[test]
fn blend_dropdown_and_opacity_field_dispatch_real_edits() {
    let fixture = fixture();
    let id = fixture.video_a;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    harness
        .get(
            By::new()
                .predicate(|n| n.author_id() == Some(format!("timeline.item.{}", id.0).as_str())),
        )
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.group.basic")))
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.tab.blending")))
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.blendMode")))
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.blendMode.multiply")))
        .click();
    harness.run_steps(2);
    assert_eq!(
        item(harness.state(), id).blend_mode,
        project::BlendMode::Multiply
    );
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.opacity")))
        .click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("50".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    let path = project::AnimationPath::new("opacity").unwrap();
    assert_eq!(
        item(harness.state(), id)
            .animations
            .value_at(&path, Tick::ZERO)
            .unwrap(),
        Some(project::ParamValue::Scalar(0.5))
    );
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(item(harness.state(), id).animations.get(&path).is_none());
    harness.state_mut().activate("edit.redo").unwrap();
    harness.run_steps(2);
    assert_eq!(
        item(harness.state(), id)
            .animations
            .value_at(&path, Tick::ZERO)
            .unwrap(),
        Some(project::ParamValue::Scalar(0.5))
    );
}

pub(crate) fn widget(harness: &Harness<'_, Editor>, id: &str) -> egui::Rect {
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

pub(crate) fn item(editor: &Editor, id: ItemId) -> &project::Item {
    editor
        .session
        .preview_project()
        .unwrap_or_else(|| editor.session.project())
        .data()
        .scene()
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
        .scene()
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
        .scene()
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
    // Read the real track/header boundary so density changes do not alter the
    // engine time targeted by the gesture.
    let body_left = widget(&harness, "track.0.header").right();
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

fn bookmark_id(editor: &Editor) -> project::BookmarkId {
    editor.session.project().data().scene().bookmarks[0].id
}

fn add_relative_keyframe(editor: &mut Editor, item: ItemId, time: Tick) {
    let path = project::AnimationPath::new("opacity").unwrap();
    editor
        .session
        .apply(Command::SetAnimatedParam {
            item,
            path: path.clone(),
            param: project::Param::new(project::ParamValue::Scalar(1.0)).unwrap(),
        })
        .unwrap();
    editor
        .session
        .apply(Command::AddKeyframe {
            item,
            path,
            component: None,
            key: project::Keyframe::new(time, project::ParamValue::Scalar(0.5)),
        })
        .unwrap();
}

fn add_key(editor: &mut Editor, item: ItemId, path: &str, time: Tick, value: f64) {
    let path = project::AnimationPath::new(path).unwrap();
    if editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .is_none_or(|candidate| candidate.animations.get(&path).is_none())
    {
        editor
            .session
            .apply(Command::SetAnimatedParam {
                item,
                path: path.clone(),
                param: project::Param::new(project::ParamValue::Scalar(value)).unwrap(),
            })
            .unwrap();
    }
    editor
        .session
        .apply(Command::AddKeyframe {
            item,
            path,
            component: None,
            key: project::Keyframe::new(time, project::ParamValue::Scalar(value)),
        })
        .unwrap();
}

fn audio_item(editor: &Editor) -> ItemId {
    editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| track.kind == TrackKind::Audio)
        .unwrap()
        .items[0]
        .id
}

#[test]
fn snapping_preference_toggles_through_app_history_and_persists() {
    let fixture = fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let project = harness.state().session.project().clone();
    let history = harness.state().session.history_len();
    assert!(harness.state().inspect().unwrap().snapping);
    assert!(harness.state().preferences.snapping);
    let snap = widget(&harness, "timeline.snap");
    click_at(&mut harness, snap.center());
    assert!(!harness.state().preferences.snapping);
    assert_eq!(harness.state().session.project(), &project);
    assert_eq!(harness.state().session.history_len(), history);
    let dir = tempfile::tempdir().unwrap();
    let store = vibecut::preferences::PreferenceStore::new(dir.path().join("preferences.json"));
    store.save(&harness.state().preferences).unwrap();
    assert!(!store.load().unwrap().snapping);
    let undo = widget(&harness, "edit.undo");
    click_at(&mut harness, undo.center());
    assert!(harness.state().preferences.snapping);
    let redo = widget(&harness, "edit.redo");
    click_at(&mut harness, redo.center());
    assert!(!harness.state().preferences.snapping);
    harness.key_press(egui::Key::N);
    harness.run_steps(2);
    assert!(harness.state().preferences.snapping);
    assert_eq!(harness.state().session.project(), &project);
}

#[test]
fn preview_axis_hover_scrubs_the_counter_and_preview_without_moving_the_playhead() {
    let fixture = fixture();
    let playhead = Tick(3 * SECOND);
    let mut editor = fixture.editor;
    editor
        .playback
        .seek(playhead, false, editor.session.project().clone())
        .unwrap();
    let mut harness = harness(editor);
    harness.run_steps(3);
    let fps = harness.state().session.project().data().canvas.fps;
    let project = serde_json::to_vec(harness.state().session.project().data()).unwrap();
    let history = harness.state().session.history_len();
    let before = harness.state().inspect().unwrap();
    assert!(before.preview_axis, "the preview axis starts on");
    assert_eq!(before.hover_time, None);
    assert_eq!(before.preview_at, None);
    assert_eq!(before.position, playhead);

    let ruler = widget(&harness, "timeline.ruler");
    let zoom = before.zoom;
    let first = egui::pos2(ruler.left() + 4.0 * zoom, ruler.center().y);
    harness.hover_at(first);
    harness.run_steps(2);
    let hovered = harness.state().inspect().unwrap();
    let at = hovered.hover_time.expect("hover time");
    assert!(
        (at.seconds() - 4.0).abs() < 0.05,
        "hover samples the pointer time, got {at:?}"
    );
    assert_eq!(hovered.preview_at, Some(at), "one frame request at hover");
    assert_eq!(
        hovered.timecode,
        time::format_timecode(at, fps).unwrap(),
        "the counter shows the hovered time"
    );
    assert_eq!(hovered.position, playhead, "the playhead does not move");
    assert_eq!(
        serde_json::to_vec(harness.state().session.project().data()).unwrap(),
        project,
        "hover never changes Project JSON"
    );
    assert_eq!(harness.state().session.history_len(), history);

    // A second move supersedes the first request instead of queueing both.
    harness.hover_at(egui::pos2(ruler.left() + 7.0 * zoom, ruler.center().y));
    harness.run_steps(2);
    let moved = harness.state().inspect().unwrap();
    let later = moved.hover_time.expect("second hover time");
    assert!(later > at, "the guide follows the pointer");
    assert_eq!(
        moved.preview_at,
        Some(later),
        "only the latest request is kept"
    );

    // Leaving the timeline returns the counter and preview to the playhead.
    let canvas = widget(&harness, "preview.canvas");
    harness.hover_at(canvas.center());
    harness.run_steps(2);
    let left = harness.state().inspect().unwrap();
    assert_eq!(left.hover_time, None);
    assert_eq!(left.preview_at, None);
    assert_eq!(
        left.timecode,
        time::format_timecode(playhead, fps).unwrap(),
        "the counter returns to the playhead"
    );
    assert_eq!(left.position, playhead);

    // A ruler click moves the playhead; the guide keeps following the pointer.
    // `drop_at` removes the cursor, so press and release in place instead.
    let click = egui::pos2(ruler.left() + 6.0 * zoom, ruler.center().y);
    harness.hover_at(click);
    harness.run_steps(1);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: click,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(1);
    }
    harness.run_steps(1);
    let clicked = harness.state().inspect().unwrap();
    assert!(
        (clicked.position.seconds() - 6.0).abs() < 0.05,
        "the ruler click sets the playhead, got {:?}",
        clicked.position
    );
    let guiding = clicked.hover_time.expect("guide after the click");
    assert_eq!(clicked.preview_at, Some(guiding));
    assert_eq!(
        clicked.timecode,
        time::format_timecode(guiding, fps).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(harness.state().session.project().data()).unwrap(),
        project
    );

    // Empty-tail pointer positions remain distinct presentation times.
    let duration = harness.state().session.project().duration();
    let mut previous = duration;
    for x in [ruler.right() - zoom, ruler.right() - 1.0] {
        harness.hover_at(egui::pos2(x, ruler.center().y));
        harness.run_steps(2);
        let tail = harness.state().inspect().unwrap();
        let at = tail.hover_time.unwrap();
        assert!(
            at > previous,
            "the guide must keep following the pointer past the end"
        );
        assert_eq!(tail.preview_at, Some(at));
        assert_eq!(tail.timecode, time::format_timecode(at, fps).unwrap());
        assert_eq!(tail.position, clicked.position);
        previous = at;
    }

    // The toolbar toggle turns the axis off; the pointer no longer scrubs.
    let toggle = widget(&harness, "timeline.preview_axis");
    click_at(&mut harness, toggle.center());
    let off = harness.state().inspect().unwrap();
    assert!(!off.preview_axis);
    assert_eq!(off.hover_time, None);
    assert_eq!(off.preview_at, None);
    assert_eq!(
        off.timecode,
        time::format_timecode(off.position, fps).unwrap()
    );
    let dir = tempfile::tempdir().unwrap();
    let store = vibecut::preferences::PreferenceStore::new(dir.path().join("preferences.json"));
    store.save(&harness.state().preferences).unwrap();
    assert!(
        !store.load().unwrap().preview_axis,
        "the toggle persists off"
    );
    harness.hover_at(first);
    harness.run_steps(2);
    let disabled = harness.state().inspect().unwrap();
    assert_eq!(disabled.hover_time, None);
    assert_eq!(disabled.preview_at, None);
    assert_eq!(
        disabled.timecode,
        time::format_timecode(disabled.position, fps).unwrap()
    );

    // `S` toggles the axis on again and hover resumes.
    harness.key_press(egui::Key::S);
    harness.run_steps(2);
    assert!(harness.state().inspect().unwrap().preview_axis);
    harness.hover_at(first);
    harness.run_steps(2);
    let resumed = harness.state().inspect().unwrap();
    assert_eq!(resumed.hover_time, resumed.preview_at);
    assert!(resumed.hover_time.is_some());
    assert_eq!(
        serde_json::to_vec(harness.state().session.project().data()).unwrap(),
        project
    );
    assert_eq!(harness.state().session.history_len(), history);
}

#[test]
fn preview_axis_tail_tracks_the_guide_counter_and_actual_layer_pixels() {
    let mut data = Project::empty(ProjectId(301), "Layer ends".into())
        .unwrap()
        .into_data();
    data.canvas.width = 64;
    data.canvas.height = 64;
    let mut session = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let main = session.project().data().scene().timeline.main.id;
    insert(
        &mut session,
        main,
        Tick::ZERO,
        SourceRange::new(Tick::ZERO, Tick(2 * SECOND)).unwrap(),
        None,
        ItemParams::Overlay(project::OverlayParams {
            color: Color([255, 0, 0, 255]),
        }),
    );
    let overlay = track(&mut session, "Overlay", TrackKind::Video);
    let blue = insert(
        &mut session,
        overlay,
        Tick::ZERO,
        SourceRange::new(Tick::ZERO, Tick(4 * SECOND)).unwrap(),
        None,
        ItemParams::Overlay(project::OverlayParams {
            color: Color([0, 0, 255, 255]),
        }),
    );
    let playhead = Tick(SECOND);
    session
        .apply(Command::SetViewState {
            zoom: 1.0,
            scroll: 0.0,
            playhead,
        })
        .unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let mut settings = render::RenderSettings::from_project(session.project());
    settings.placements.insert(
        blue,
        render::Placement {
            destination: render::Rect {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
            ..Default::default()
        },
    );
    playback.set_render_settings(settings).unwrap();
    playback
        .seek(playhead, false, session.project().clone())
        .unwrap();
    let mut harness = harness(Editor::new(session, playback, Preferences::default()));
    harness.run_steps(3);
    let project = serde_json::to_vec(harness.state().session.project().data()).unwrap();
    let history = harness.state().session.history_len();
    let ruler = widget(&harness, "timeline.ruler");
    let body = widget(&harness, "timeline.marquee");
    let zoom = harness.state().inspect().unwrap().zoom;
    let fps = harness.state().session.project().data().canvas.fps;
    for (seconds, y, left, right) in [
        (1, ruler.center().y, egui::Color32::RED, egui::Color32::BLUE),
        (
            2,
            body.center().y,
            egui::Color32::BLACK,
            egui::Color32::BLUE,
        ),
        (
            3,
            body.center().y,
            egui::Color32::BLACK,
            egui::Color32::BLUE,
        ),
        (
            4,
            ruler.center().y,
            egui::Color32::BLACK,
            egui::Color32::BLACK,
        ),
        (
            5,
            ruler.center().y,
            egui::Color32::BLACK,
            egui::Color32::BLACK,
        ),
        (
            6,
            body.center().y,
            egui::Color32::BLACK,
            egui::Color32::BLACK,
        ),
    ] {
        let at = Tick(seconds * SECOND);
        let pointer = egui::pos2(ruler.left() + seconds as f32 * zoom, y);
        harness.hover_at(pointer);
        harness.run_steps(2);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while harness.state().presented_frame() != Some(at) {
            assert!(
                std::time::Instant::now() < deadline,
                "actual hover frame did not arrive"
            );
            harness.run_steps(1);
            std::thread::yield_now();
        }
        let inspect = harness.state().inspect().unwrap();
        assert_eq!(inspect.hover_time, Some(at));
        assert_eq!(inspect.timecode, time::format_timecode(at, fps).unwrap());
        assert_eq!(inspect.position, playhead);
        let x = pointer.x.round() + 0.5;
        assert!(
            painted_shapes(&harness).iter().any(|shape| {
                matches!(shape, egui::Shape::LineSegment { points, stroke }
                if points[0].x == x && points[1].x == x
                && (points[0].y - ruler.top()).abs() < 1.0
                && (points[1].y - body.bottom()).abs() < 1.0
                && stroke.width == 1.0)
            }),
            "actual painted guide must follow the pointer at {seconds}s"
        );
        let image = &harness.state().playback.frame().unwrap().image;
        assert_eq!(image.pixels[32 * 64 + 16], left);
        assert_eq!(image.pixels[32 * 64 + 48], right);
        if seconds > 4 {
            assert!(
                image
                    .pixels
                    .iter()
                    .all(|pixel| *pixel == egui::Color32::BLACK),
                "globally empty tail must contain no final-clip pixels"
            );
        }
        assert!(!harness.state().playback.playing());
    }
    // Each exit path restores actual playhead pixels, independently of the
    // counter metadata. Fullscreen is exercised offscreen, without a window.
    for exit in ["leave", "off", "fullscreen"] {
        if !harness.state().preferences.preview_axis {
            harness
                .state_mut()
                .activate("timeline.preview_axis")
                .unwrap();
        }
        harness.hover_at(egui::pos2(ruler.left() + 6.0 * zoom, body.center().y));
        harness.run_steps(2);
        match exit {
            "leave" => {
                let canvas = widget(&harness, "preview.canvas");
                harness.hover_at(canvas.center());
            }
            "off" => harness
                .state_mut()
                .activate("timeline.preview_axis")
                .unwrap(),
            _ => harness.state_mut().activate("preview.fullscreen").unwrap(),
        }
        harness.run_steps(3);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while harness.state().presented_frame() != Some(playhead) {
            assert!(std::time::Instant::now() < deadline);
            harness.run_steps(1);
            std::thread::yield_now();
        }
        let inspect = harness.state().inspect().unwrap();
        assert_eq!(inspect.hover_time, None);
        assert_eq!(
            inspect.timecode,
            time::format_timecode(playhead, fps).unwrap()
        );
        let image = &harness.state().playback.frame().unwrap().image;
        assert_eq!(image.pixels[32 * 64 + 16], egui::Color32::RED);
        assert_eq!(image.pixels[32 * 64 + 48], egui::Color32::BLUE);
    }
    assert_eq!(
        serde_json::to_vec(harness.state().session.project().data()).unwrap(),
        project
    );
    assert_eq!(harness.state().session.history_len(), history);
}

#[test]
fn preview_axis_hover_renders_screenshot() {
    let fixture = fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let ruler = widget(&harness, "timeline.ruler");
    let zoom = harness.state().inspect().unwrap().zoom;
    harness.hover_at(egui::pos2(ruler.left() + 4.0 * zoom, ruler.center().y));
    harness.run_steps(2);
    assert!(
        harness.state().inspect().unwrap().hover_time.is_some(),
        "the guide and counter need a hover time"
    );
    let image = harness.render().unwrap();
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    let path = target.join("p1-300-preview-axis.png");
    image.save(&path).unwrap();
    eprintln!("preview axis screenshot: {}", path.display());
}

#[test]
fn move_and_trim_snap_and_shift_mid_drag_bypasses() {
    let fixture = fixture();
    let video_a = fixture.video_a;
    let video_b = fixture.video_b;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let zoom = harness.state().inspect().unwrap().zoom;
    let body = widget(&harness, "timeline.marquee");
    let item_rect = widget(&harness, &format!("timeline.item.{}", video_b.0));
    let target = egui::pos2(item_rect.center().x + 2.1 * zoom, item_rect.center().y);
    harness.drag_at(item_rect.center());
    harness.run_steps(1);
    harness.hover_at(target);
    harness.run_steps(2);
    assert_eq!(item(harness.state(), video_b).start, Tick(8 * SECOND));
    assert_eq!(
        harness.state().inspect().unwrap().snap_indicator,
        Some(Tick(8 * SECOND))
    );
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    harness.run_steps(2);
    assert!(
        (item(harness.state(), video_b).start.seconds() - 8.1).abs() < 0.01,
        "Shift must bypass snapping mid-drag"
    );
    assert_eq!(harness.state().inspect().unwrap().snap_indicator, None);
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run_steps(2);
    assert_eq!(item(harness.state(), video_b).start, Tick(8 * SECOND));
    harness.drop_at(target);
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);

    let handle = widget(&harness, &format!("timeline.item.{}.trim_end", video_a.0));
    let target = egui::pos2(body.left() + 5.15 * zoom, handle.center().y);
    harness.drag_at(handle.center());
    harness.run_steps(1);
    harness.hover_at(target);
    harness.run_steps(2);
    assert_eq!(
        item(harness.state(), video_a).end().unwrap(),
        Tick(5 * SECOND)
    );
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    harness.run_steps(2);
    assert!(
        (item(harness.state(), video_a).end().unwrap().seconds() - 5.15).abs() < 0.01,
        "Shift must bypass trim snapping mid-drag"
    );
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run_steps(2);
    assert_eq!(
        item(harness.state(), video_a).end().unwrap(),
        Tick(5 * SECOND)
    );
    harness.drop_at(target);
    harness.run_steps(2);

    // The bookmark at eight seconds is beyond the neighbour at six seconds.
    // Clamping the trim must hide a target the edge cannot actually reach.
    let handle = widget(&harness, &format!("timeline.item.{}.trim_end", video_a.0));
    let target = egui::pos2(body.left() + 7.9 * zoom, handle.center().y);
    harness.drag_at(handle.center());
    harness.run_steps(1);
    harness.hover_at(target);
    harness.run_steps(2);
    assert_eq!(
        item(harness.state(), video_a).end().unwrap(),
        Tick(6 * SECOND)
    );
    assert_eq!(harness.state().inspect().unwrap().snap_indicator, None);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
}

#[test]
fn multi_select_move_snaps_one_edge_and_keeps_group_spacing() {
    let fixture = fixture();
    let video_b = fixture.video_b;
    let text = text_item_id(&fixture.editor);
    let mut editor = fixture.editor;
    editor.selection = [video_b, text].into_iter().collect();
    let mut harness = harness(editor);
    harness.run_steps(3);
    let zoom = harness.state().inspect().unwrap().zoom;
    let item_rect = widget(&harness, &format!("timeline.item.{}", video_b.0));
    let target = egui::pos2(item_rect.center().x + 1.9 * zoom, item_rect.center().y);
    drag(&mut harness, item_rect.center(), target);
    assert_eq!(item(harness.state(), video_b).start, Tick(8 * SECOND));
    assert_eq!(
        item(harness.state(), video_b).start - item(harness.state(), text).start,
        Tick(4 * SECOND),
        "the selected group keeps its entry spacing"
    );
}

#[test]
fn trim_snaps_to_an_animation_keyframe() {
    let mut fixture = fixture();
    let video_a = fixture.video_a;
    let text = text_item_id(&fixture.editor);
    add_relative_keyframe(&mut fixture.editor, text, Tick(2 * SECOND));
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let zoom = harness.state().inspect().unwrap().zoom;
    let body = widget(&harness, "timeline.marquee");
    let handle = widget(&harness, &format!("timeline.item.{}.trim_end", video_a.0));
    drag(
        &mut harness,
        handle.center(),
        egui::pos2(body.left() + 4.1 * zoom, handle.center().y),
    );
    assert_eq!(
        item(harness.state(), video_a).end().unwrap(),
        Tick(4 * SECOND),
        "the text keyframe at 4 s is a snap source"
    );
}

#[test]
fn bookmark_drag_snaps_shift_bypasses_and_commits_one_undo_step() {
    let fixture = fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let history = harness.state().session.history_len();
    let id = bookmark_id(harness.state());
    let zoom = harness.state().inspect().unwrap().zoom;
    let body = widget(&harness, "timeline.marquee");
    let marker = widget(&harness, &format!("timeline.bookmark.{}", id.0));
    let target = egui::pos2(body.left() + 9.9 * zoom, marker.center().y);
    harness.drag_at(marker.center());
    harness.run_steps(1);
    harness.hover_at(target);
    harness.run_steps(2);
    assert_eq!(
        harness.state().session.project().data().scene().bookmarks[0].start,
        Tick(10 * SECOND)
    );
    assert_eq!(
        harness.state().inspect().unwrap().snap_indicator,
        Some(Tick(10 * SECOND))
    );
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    harness.run_steps(2);
    assert!(
        (harness.state().session.project().data().scene().bookmarks[0]
            .start
            .seconds()
            - 9.9)
            .abs()
            < 0.01,
        "Shift must bypass bookmark snapping mid-drag"
    );
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run_steps(2);
    assert_eq!(
        harness.state().session.project().data().scene().bookmarks[0].start,
        Tick(10 * SECOND)
    );
    assert_eq!(
        harness.state().session.history_len(),
        history,
        "bookmark preview is not committed while dragging"
    );
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(
        harness.state().session.project().data().scene().bookmarks[0].start,
        Tick(8 * SECOND),
        "Escape cancels the bookmark drag"
    );
    assert_eq!(harness.state().inspect().unwrap().snap_indicator, None);
    harness.drop_at(target);
    harness.run_steps(2);
    let marker = widget(&harness, &format!("timeline.bookmark.{}", id.0));
    harness.drag_at(marker.center());
    harness.run_steps(1);
    harness.hover_at(target);
    harness.run_steps(2);
    harness.drop_at(target);
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(
        harness.state().session.project().data().scene().bookmarks[0].start,
        Tick(8 * SECOND)
    );
}

fn bookmark_at(editor: &Editor, start: Tick) -> Option<project::BookmarkId> {
    editor
        .session
        .project()
        .data()
        .scene()
        .bookmarks
        .iter()
        .find(|bookmark| bookmark.start == start)
        .map(|bookmark| bookmark.id)
}

fn bookmark(editor: &Editor, id: project::BookmarkId) -> project::Bookmark {
    editor
        .session
        .project()
        .data()
        .scene()
        .bookmarks
        .iter()
        .find(|bookmark| bookmark.id == id)
        .unwrap()
        .clone()
}

#[test]
fn bookmark_toggle_and_popover_fields_are_one_undo_step_each() {
    let mut fixture = fixture();
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
    let history = harness.state().session.history_len();

    // The toolbar and the shortcut both run bookmark.toggle at the playhead.
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark")))
        .click();
    harness.run_steps(2);
    assert!(bookmark_at(harness.state(), Tick(3 * SECOND)).is_some());
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark")))
        .click();
    harness.run_steps(2);
    assert!(bookmark_at(harness.state(), Tick(3 * SECOND)).is_none());
    assert_eq!(harness.state().session.history_len().0, history.0 + 2);
    harness.key_press(egui::Key::M);
    harness.run_steps(2);
    let id = bookmark_at(harness.state(), Tick(3 * SECOND)).expect("M adds a bookmark");
    assert_eq!(harness.state().session.history_len().0, history.0 + 3);

    // Opening the editor seeks but changes no project byte and no history.
    let bytes = serde_json::to_vec(harness.state().session.project().data()).unwrap();
    let marker = widget(&harness, &format!("timeline.bookmark.{}", id.0));
    click_at(&mut harness, marker.center());
    assert_eq!(
        harness.state().inspect().unwrap().bookmark_popover,
        Some(id),
        "clicking the marker opens its editor"
    );
    assert_eq!(
        serde_json::to_vec(harness.state().session.project().data()).unwrap(),
        bytes,
        "opening the editor must not mutate the project"
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 3);

    // Each edited field is exactly one bookmark.update.
    let before = harness.state().session.history_len().0;
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark.note")))
        .click();
    harness.run_steps(1);
    harness.event(egui::Event::Text("Intro".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(bookmark(harness.state(), id).note.as_str(), "Intro");
    assert_eq!(harness.state().session.history_len().0, before + 1);

    let before = harness.state().session.history_len().0;
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark.duration")))
        .click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("0.5".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert!((bookmark(harness.state(), id).duration.seconds() - 0.5).abs() < 1e-6);
    assert_eq!(harness.state().session.history_len().0, before + 1);
    harness
        .get(
            By::new()
                .predicate(|node| node.author_id() == Some("timeline.bookmark.duration.increase")),
        )
        .click();
    harness.run_steps(2);
    assert!(
        (bookmark(harness.state(), id).duration.seconds() - 0.6).abs() < 1e-6,
        "the stepper commits one 0.1 s step"
    );
    assert_eq!(harness.state().session.history_len().0, before + 2);

    // The engine clamps an oversized duration to the scene tail.
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark.duration")))
        .click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("999".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert!(
        (bookmark(harness.state(), id).duration.seconds() - 7.0).abs() < 1e-6,
        "a 999 s duration clamps to the 7 s left in the scene"
    );
    assert_eq!(harness.state().session.history_len().0, before + 3);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark.duration")))
        .click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("0.6".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);

    let before = harness.state().session.history_len().0;
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark.color")))
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("color.swatch.0")))
        .click();
    harness.run_steps(3);
    assert_ne!(
        bookmark(harness.state(), id).color,
        engine::DEFAULT_BOOKMARK_COLOR
    );
    assert_eq!(harness.state().session.history_len().0, before + 1);
    // Escape closes the colour window before the marker editor.
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(
        harness.state().inspect().unwrap().bookmark_popover,
        Some(id)
    );
    assert_eq!(harness.state().session.history_len().0, before + 1);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark.color.reset")))
        .click();
    harness.run_steps(2);
    assert_eq!(
        bookmark(harness.state(), id).color,
        engine::DEFAULT_BOOKMARK_COLOR
    );
    assert_eq!(harness.state().session.history_len().0, before + 2);

    let image = harness.render().unwrap();
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    let path = target.join("p1-49-bookmarks.png");
    image.save(&path).unwrap();
    eprintln!("bookmark screenshot: {}", path.display());

    // Delete is one bookmark.remove and closes the editor.
    let before = harness.state().session.history_len().0;
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark.delete")))
        .click();
    harness.run_steps(2);
    assert!(bookmark_at(harness.state(), Tick(3 * SECOND)).is_none());
    assert_eq!(harness.state().inspect().unwrap().bookmark_popover, None);
    assert_eq!(harness.state().session.history_len().0, before + 1);

    // Escape closes the editor without touching the project.
    let eight = bookmark_at(harness.state(), Tick(8 * SECOND)).unwrap();
    let marker = widget(&harness, &format!("timeline.bookmark.{}", eight.0));
    click_at(&mut harness, marker.center());
    assert_eq!(
        harness.state().inspect().unwrap().bookmark_popover,
        Some(eight)
    );
    let bytes = serde_json::to_vec(harness.state().session.project().data()).unwrap();
    let history = harness.state().session.history_len();
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark.note")))
        .click();
    harness.run_steps(1);
    harness.event(egui::Event::Text("Draft".into()));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().inspect().unwrap().bookmark_popover, None);
    assert_eq!(
        serde_json::to_vec(harness.state().session.project().data()).unwrap(),
        bytes,
        "Escape discards the uncommitted note"
    );
    assert_eq!(harness.state().session.history_len(), history);

    // Switching to another marker commits the draft that loses focus.
    let project = harness.state().session.project().clone();
    harness
        .state_mut()
        .playback
        .seek(Tick(5 * SECOND), false, project)
        .unwrap();
    harness.state_mut().activate("timeline.bookmark").unwrap();
    harness.run_steps(2);
    let five = bookmark_at(harness.state(), Tick(5 * SECOND)).unwrap();
    let marker = widget(&harness, &format!("timeline.bookmark.{}", eight.0));
    click_at(&mut harness, marker.center());
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("timeline.bookmark.note")))
        .click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("Switch".into()));
    harness.run_steps(1);
    let before = harness.state().session.history_len();
    let marker = widget(&harness, &format!("timeline.bookmark.{}", five.0));
    click_at(&mut harness, marker.center());
    assert_eq!(bookmark(harness.state(), eight).note.as_str(), "Switch");
    assert_eq!(
        harness.state().inspect().unwrap().bookmark_popover,
        Some(five)
    );
    assert_eq!(harness.state().session.history_len().0, before.0 + 1);
}

#[test]
fn drag_preview_keeps_committed_history_and_indicator_until_commit() {
    let fixture = fixture();
    let video_b = fixture.video_b;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let committed = harness.state().session.project().clone();
    let before = serde_json::to_vec(committed.data()).unwrap();
    let history = harness.state().session.history_len();
    let zoom = harness.state().inspect().unwrap().zoom;
    let item_rect = widget(&harness, &format!("timeline.item.{}", video_b.0));
    harness.drag_at(item_rect.center());
    harness.run_steps(1);
    harness.hover_at(egui::pos2(
        item_rect.center().x + 0.5 * zoom,
        item_rect.center().y,
    ));
    harness.run_steps(2);
    assert_eq!(harness.state().inspect().unwrap().snap_indicator, None);
    harness.hover_at(egui::pos2(
        item_rect.center().x + 2.1 * zoom,
        item_rect.center().y,
    ));
    harness.run_steps(2);
    assert_eq!(
        harness.state().inspect().unwrap().snap_indicator,
        Some(Tick(8 * SECOND))
    );
    assert!(harness.state().session.gesture_active());
    assert_eq!(serde_json::to_vec(committed.data()).unwrap(), before);
    assert_eq!(
        harness.state().session.project(),
        &committed,
        "the committed snapshot is untouched until commit"
    );
    assert_ne!(
        harness.state().session.preview_project().unwrap(),
        &committed,
        "the staged candidate carries the move"
    );
    assert_eq!(
        harness.state().inspect().unwrap().preview,
        Some("timeline.move_items")
    );
    assert_eq!(
        harness.state().session.history_len(),
        history,
        "the preview gesture is not committed while dragging"
    );
    let image = harness.render().unwrap();
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    let path = target.join("p1-21-snap.png");
    image.save(&path).unwrap();
    eprintln!("snap screenshot: {}", path.display());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(
        serde_json::to_vec(harness.state().session.project().data()).unwrap(),
        before,
        "cancel restores the committed project bytes"
    );
    assert_eq!(harness.state().session.history_len(), history);
    assert_eq!(harness.state().inspect().unwrap().snap_indicator, None);
    harness.drag_at(item_rect.center());
    harness.run_steps(1);
    harness.hover_at(egui::pos2(
        item_rect.center().x + 2.1 * zoom,
        item_rect.center().y,
    ));
    harness.run_steps(2);
    harness.drop_at(egui::pos2(
        item_rect.center().x + 2.1 * zoom,
        item_rect.center().y,
    ));
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert_ne!(
        serde_json::to_vec(harness.state().session.project().data()).unwrap(),
        before
    );
}

fn item_opt(editor: &Editor, id: ItemId) -> Option<&project::Item> {
    editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == id)
}

#[test]
fn every_editor_button_reports_its_embedded_icon() {
    let mut harness = harness(fixture().editor);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        harness.run_steps(1);
        if harness
            .state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .any(|widget| widget.id == "preview.retry")
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "preview failure did not reach the retry state"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // Observe both real popovers: opening one closes the other.
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("window.menu")))
        .click();
    harness.run_steps(3);
    let mut widgets = harness.state().inspect().unwrap().widgets;
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("preview.menu")))
        .click();
    harness.run_steps(3);
    for widget in harness.state().inspect().unwrap().widgets {
        if let Some(previous) = widgets.iter_mut().find(|previous| previous.id == widget.id) {
            *previous = widget;
        } else {
            widgets.push(widget);
        }
    }

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
        ("preview.menu", "list-bullets"),
        ("preview.zoom", "magnifying-glass-plus"),
        ("preview.fullscreen", "corners-out"),
        ("transport.forward", "skip-forward"),
        ("preview.retry", "arrow-clockwise"),
        ("edit.undo", "arrow-counter-clockwise"),
        ("edit.redo", "arrow-clockwise"),
        ("timeline.split", "scissors"),
        ("timeline.delete", "trash"),
        ("timeline.snap", "magnet"),
        ("timeline.magnet", "link-simple"),
        ("timeline.ripple", "waveform"),
        ("timeline.preview_axis", "arrows-in-line-vertical"),
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
        (split.rect.height() - 24.0).abs() < 0.5 && (split.rect.width() - 24.0).abs() < 0.5,
        "timeline actions use the measured 24x24 icon-only hit box, got {:?}",
        split.rect
    );
    let timeline = harness.state().panels()["timeline"];
    assert!(
        (split.rect.min.y - timeline.top() - 4.0).abs() < 0.5,
        "timeline toolbar uses its measured 32 px row within the full panel slot"
    );
    let undo = format!(
        "Undo ({})",
        vibecut::actions::Chord::parse("Ctrl+Z")
            .unwrap()
            .display(vibecut::actions::Platform::current())
    );
    for (id, text) in [
        ("edit.undo", undo.as_str()),
        ("timeline.split", "Split (Ctrl+B)"),
        ("timeline.snap", "Snap (N)"),
        ("timeline.preview_axis", "Turn off preview axis (S)"),
        ("timeline.zoom_in", "Zoom +"),
        ("timeline.magnet", "Main track magnet"),
        ("transport.back", "Back 1 second (J)"),
        ("transport.play", "Play (Space)"),
        ("preview.retry", "Retry"),
    ] {
        assert_eq!(tooltip(id), Some(text), "{id} tooltip");
    }
}

fn text_item_id(editor: &Editor) -> ItemId {
    editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| matches!(item.content, ItemParams::Text(_)))
        .unwrap()
        .id
}

/// The track holding `item`, copied so callers can mutate the session.
fn item_track(editor: &Editor, item: ItemId) -> TrackId {
    editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| track.items.iter().any(|candidate| candidate.id == item))
        .unwrap()
        .id
}

#[test]
fn delete_invalidates_preview_and_clamps_playhead() {
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
fn offscreen_drop_closes_gesture() {
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
fn group_move_preserves_spacing_at_timeline_zero() {
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
fn move_back_to_origin_restores_position() {
    for snapping in [true, false] {
        let fixture = fixture();
        let id = fixture.video_b;
        let mut editor = fixture.editor;
        editor.preferences.snapping = snapping;
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
fn trim_release_survives_its_handle_becoming_narrow() {
    let fixture = fixture();
    let id = fixture.video_a;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let zoom = harness.state().inspect().unwrap().zoom;
    // Read the real track/header boundary so density changes do not alter the
    // engine time targeted by the gesture.
    let body_left = widget(&harness, "track.0.header").right();
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
fn split_respects_selection_outside_playhead() {
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
        .apply(Command::SetViewState {
            zoom: 5.0,
            scroll: 80.0,
            playhead: Tick::ZERO,
        })
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
        .apply(Command::SetViewState {
            zoom: 2.0,
            scroll: 40.0,
            playhead: Tick(3 * SECOND),
        })
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
        .apply(Command::SetViewState {
            zoom: 3.0,
            scroll: 0.0,
            playhead: Tick::ZERO,
        })
        .unwrap();
    fixture.editor.activate("timeline.zoom_in").unwrap();
    assert!(
        (fixture.editor.inspect().unwrap().zoom - 204.0).abs() < 0.001,
        "a relative UI action starts from the latest engine-issued view"
    );
    // An adapter or scene restoration replaces the navigation context, including
    // the transient pre-anchor position from an earlier zoom gesture.
    for (zoom, scroll) in [(1.0, 80.0), (25.0, 2000.0)] {
        fixture
            .editor
            .session
            .apply(Command::SetViewState {
                zoom,
                scroll,
                playhead: Tick::ZERO,
            })
            .unwrap();
        fixture.editor.activate("transport.pause").unwrap();
        if zoom == 1.0 {
            fixture.editor.activate("timeline.zoom_in").unwrap();
        }
    }
    for _ in 0..8 {
        fixture.editor.activate("timeline.zoom_out").unwrap();
    }
    assert!(
        (fixture.editor.session.metadata().view.scroll - 2000.0).abs() < 0.01,
        "external view restoration must discard the previous context's pre-anchor scroll"
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
            mask: "mask1".into(),
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
        .scene()
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
        .scene()
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
            ..Default::default()
        }),
    );
    (fixture, first, second)
}

#[test]
fn mixed_text_background_is_reported_as_indeterminate() {
    let (mut fixture, first, second) = two_text_fixture();
    fixture
        .editor
        .session
        .apply(Command::SetItemParamMany {
            items: vec![second],
            patch: engine::ItemParamPatch::TextBackground(Some(Default::default())),
        })
        .unwrap();
    fixture.editor.selection = [first, second].into_iter().collect();
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1600.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
    harness.run_steps(3);
    let checkbox = harness
        .get(By::new().predicate(|node| node.author_id() == Some("inspector.text_background")));
    assert_eq!(
        egui_kittest::kittest::NodeT::accesskit_node(&checkbox).toggled(),
        Some(egui::accesskit::Toggled::Mixed)
    );
}

#[test]
fn mixed_text_inspector_applies_one_field_and_restores_selected_items() {
    let (fixture, first, second) = two_text_fixture();
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1600.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
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
    let fields = vibecut::shared_fields(
        harness.state().session.project(),
        &harness.state().selection,
    );
    assert_eq!(fields.len(), 9);
    for field in &fields {
        assert_eq!(field.compatible, 2);
        assert_eq!(
            field.value.is_none(),
            matches!(field.id, "text_content" | "text_size" | "text_color"),
            "{} mixed state",
            field.id
        );
    }
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
fn text_insert_button_fits_the_media_panel_in_both_locales() {
    for locale in [Locale::En, Locale::Vi] {
        for width in [1280.0, 1440.0] {
            let mut fixture = fixture();
            fixture.editor.preferences.locale = locale;
            let mut harness = crate::gpu::builder()
                .with_size(egui::vec2(width, 900.0))
                .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
            harness.run_steps(3);
            let button = widget(&harness, "text.insert");
            assert!(
                harness.state().panels()["media"].contains_rect(button),
                "{locale:?} at {width}: {button:?} outside {:?}",
                harness.state().panels()["media"]
            );
            let before = count(harness.state());
            let project = harness.state().session.project().clone();
            let history = harness.state().session.history_len().0;
            harness
                .get(By::new().predicate(|node| node.author_id() == Some("text.insert")))
                .click();
            harness.run_steps(2);
            assert_eq!(count(harness.state()), before + 1);
            assert_eq!(harness.state().session.history_len().0, history + 1);
            harness
                .get(By::new().predicate(|node| node.author_id() == Some("edit.undo")))
                .click();
            harness.run_steps(2);
            assert_eq!(harness.state().session.project(), &project);
        }
    }
}

#[test]
fn text_position_controls_are_reachable_in_a_normal_height_window() {
    let fixture = fixture();
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
    harness.run_steps(3);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("text.insert")))
        .click();
    harness.run_steps(3);
    let panel = harness.state().panels()["inspector"];
    let scroll =
        (panel.bottom() - widget(&harness, "inspector.text_position_y").bottom() - 8.0).min(0.0);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(panel.center()));
    harness.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, scroll),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(20);
    assert!(panel.contains_rect(widget(&harness, "inspector.text_position_y")));
}

#[test]
fn text_insert_and_style_controls_use_one_undo_step_per_edit() {
    let mut fixture = fixture();
    let before = fixture.editor.session.project().clone();
    let items_before = count(&fixture.editor);
    let history = fixture.editor.session.history_len().0;
    fixture
        .editor
        .playback
        .seek(Tick(7 * SECOND), false, before.clone())
        .unwrap();
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1600.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
    harness.run_steps(3);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("text.insert")))
        .click();
    harness.run_steps(2);
    assert_eq!(count(harness.state()), items_before + 1);
    assert_eq!(harness.state().session.history_len().0, history + 1);
    let selected = harness.state().inspect().unwrap().selection;
    assert_eq!(selected.len(), 1);
    let item_view = item(harness.state(), selected[0]);
    assert_eq!(item_view.start, Tick(7 * SECOND));
    assert_eq!(item_view.source_range.duration(), Tick(5 * SECOND));
    let ItemParams::Text(text) = &item_view.content else {
        panic!("inserted text item")
    };
    assert_eq!(text.font.as_str(), "Inter");
    assert_eq!(text.font_size, 48.0);
    assert_eq!(text.background, None);
    assert_eq!(text.position, project::TextPosition::default());
    let text_track_count = harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .filter(|track| track.kind == TrackKind::Text)
        .count();
    assert_eq!(text_track_count, 1, "free space reuses the text track");

    let history = harness.state().session.history_len().0;
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.text_size")))
        .click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(1);
    harness.event(egui::Event::Text("64".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(harness.state().session.history_len().0, history + 1);
    let style_history = harness.state().session.history_len().0;
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.text_font")))
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("fonts.search")))
        .click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("Noto Serif".into()));
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.text_font.Noto Serif")))
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.text_background")))
        .click();
    harness.run_steps(2);
    for (field, value) in [
        ("text_content", "Xin chào"),
        ("text_color", "#11223344"),
        ("text_background_color", "#01020380"),
        ("text_position_x", "25"),
        ("text_position_y", "30"),
    ] {
        let id = format!("inspector.{field}");
        harness
            .get(By::new().predicate(|n| n.author_id() == Some(id.as_str())))
            .click();
        harness.run_steps(1);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.event(egui::Event::Text(value.into()));
        harness.run_steps(1);
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
    }
    assert_eq!(
        harness.state().session.history_len().0,
        style_history + 7,
        "each committed field edit is one undo step"
    );
    let ItemParams::Text(text) = &item(harness.state(), selected[0]).content else {
        panic!("text item")
    };
    assert_eq!(text.content.as_str(), "Xin chào");
    assert_eq!(text.font.as_str(), "Noto Serif");
    assert_eq!(text.font_size, 64.0);
    assert_eq!(text.color, Color([17, 34, 51, 68]));
    assert_eq!(
        text.background.as_ref().unwrap().color,
        Color([1, 2, 3, 128])
    );
    assert_eq!(text.position, project::TextPosition { x: 25.0, y: 30.0 });
    for (field, value) in [
        ("text_font", ""),
        ("text_size", "4097"),
        ("text_position_x", "101"),
        ("text_position_y", "NaN"),
        ("text_background", "not-a-colour"),
    ] {
        assert!(
            harness.state_mut().set_shared_field(field, value).is_err(),
            "{field} accepted {value}"
        );
    }

    let encoded = project::encode(harness.state().session.project(), false).unwrap();
    let decoded = project::decode(&encoded).unwrap();
    assert_eq!(decoded.project, **harness.state().session.project());

    for _ in 0..9 {
        harness.state_mut().activate("edit.undo").unwrap();
    }
    assert_eq!(harness.state().session.project(), &before);
    for _ in 0..9 {
        harness.state_mut().activate("edit.redo").unwrap();
    }
    let ItemParams::Text(text) = &item(harness.state(), selected[0]).content else {
        panic!("text item")
    };
    assert_eq!(text.content.as_str(), "Xin chào");
}

#[test]
fn mixed_audio_mute_shows_indeterminate_and_undo_restores_each_item() {
    let mut fixture = fixture();
    let audio_track = &fixture
        .editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .audio[0];
    let track_id = audio_track.id;
    let first = audio_track.items[0].id;
    let second = insert(
        &mut fixture.editor.session,
        track_id,
        Tick(10 * SECOND),
        SourceRange::new(Tick::ZERO, Tick(10 * SECOND)).unwrap(),
        Some(AssetId(2)),
        ItemParams::Audio(project::AudioParams {
            fades: Default::default(),
            volume: -6.0,
            muted: true,
            source: Default::default(),
        }),
    );
    let video = fixture.video_a;
    fixture.editor.selection = [first, second, video].into_iter().collect();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let before = harness.state().session.project().clone();
    let history = harness.state().session.history_len().0;
    let mute = By::new().predicate(|n| n.author_id() == Some("inspector.audio_muted"));
    assert_eq!(
        harness.get(mute.clone()).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::Mixed)
    );
    harness.get(mute.clone()).click();
    harness.run_steps(2);
    assert_eq!(
        harness.get(mute.clone()).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    assert_eq!(harness.state().session.history_len().0, history + 1);
    for (id, gain) in [(first, 0.0), (second, -6.0)] {
        let ItemParams::Audio(params) = &item(harness.state(), id).content else {
            panic!("audio")
        };
        assert!(params.muted);
        assert_eq!(params.volume, gain);
    }
    let ItemParams::Video(params) = &item(harness.state(), video).content else {
        panic!("video")
    };
    assert!(params.muted);
    assert_eq!(params.volume, 0.0);
    let after = harness.state().session.project().clone();
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(harness.state().session.project(), &before);
    assert_eq!(
        harness.get(mute.clone()).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::Mixed)
    );
    harness.state_mut().activate("edit.redo").unwrap();
    harness.run_steps(2);
    assert_eq!(harness.state().session.project(), &after);
    harness.get(mute.clone()).click();
    harness.run_steps(2);
    assert_eq!(
        harness.get(mute).accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    let ItemParams::Video(params) = &item(harness.state(), video).content else {
        panic!("video")
    };
    assert!(!params.muted);
    for id in [first, second] {
        let ItemParams::Audio(params) = &item(harness.state(), id).content else {
            panic!("audio")
        };
        assert!(!params.muted);
    }
}

#[test]
fn audio_mute_and_volume_controls_commit_one_step_and_persist() {
    let fixture = fixture();
    let audio = fixture
        .editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|t| t.kind == TrackKind::Audio)
        .unwrap()
        .items[0]
        .id;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    harness
        .get(
            By::new().predicate(|n| {
                n.author_id() == Some(format!("timeline.item.{}", audio.0).as_str())
            }),
        )
        .click();
    harness.run_steps(2);
    let history = harness.state().session.history_len().0;
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.audio_muted")))
        .click();
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history + 1);
    let ItemParams::Audio(params) = &item(harness.state(), audio).content else {
        panic!("audio")
    };
    assert!(params.muted);
    assert_eq!(params.volume, 0.0);
    harness.state_mut().activate("edit.undo").unwrap();
    let ItemParams::Audio(params) = &item(harness.state(), audio).content else {
        panic!("audio")
    };
    assert!(!params.muted);
    harness.state_mut().activate("edit.redo").unwrap();
    let ItemParams::Audio(params) = &item(harness.state(), audio).content else {
        panic!("audio")
    };
    assert!(params.muted);

    harness
        .state_mut()
        .set_shared_field("audio_gain", "-6")
        .unwrap();
    let ItemParams::Audio(params) = &item(harness.state(), audio).content else {
        panic!("audio")
    };
    assert_eq!(params.volume, -6.0);
    assert!(params.muted);
    for invalid in ["NaN", "25", "-121", "loud", ""] {
        assert!(
            harness
                .state_mut()
                .set_shared_field("audio_gain", invalid)
                .is_err(),
            "gain accepted {invalid}"
        );
    }
    for invalid in ["maybe", "2"] {
        assert!(
            harness
                .state_mut()
                .set_shared_field("audio_muted", invalid)
                .is_err(),
            "mute accepted {invalid}"
        );
    }
    let encoded = project::encode(harness.state().session.project(), false).unwrap();
    let decoded = project::decode(&encoded).unwrap();
    assert_eq!(decoded.project, **harness.state().session.project());
    harness
        .state_mut()
        .set_shared_field("audio_muted", "false")
        .unwrap();
    let ItemParams::Audio(params) = &item(harness.state(), audio).content else {
        panic!("audio")
    };
    assert_eq!(params.volume, -6.0);
    assert!(!params.muted);
}

#[test]
fn compatible_audio_gain_inspector_edits_video_and_preserves_mute() {
    let fixture = fixture();
    let video = fixture.video_a;
    let audio = fixture
        .editor
        .session
        .project()
        .data()
        .scene()
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
    let ItemParams::Video(video_params) = &item(harness.state(), video).content else {
        panic!("video")
    };
    assert_eq!(video_params.volume, -6.0);
    assert!(!video_params.muted);
    let ItemParams::Audio(p) = &item(harness.state(), audio).content else {
        panic!("audio")
    };
    assert_eq!(p.volume, -6.0);
    assert!(!p.muted);
}

#[test]
fn drop_at_track_edges_creates_one_track_and_one_undo_step() {
    for edge in [Some(false), Some(true), None] {
        let fixture = fixture();
        let moving = if edge.is_some() {
            fixture.video_a
        } else {
            fixture.video_b
        };
        let mut h = crate::gpu::builder()
            .with_size(egui::vec2(1728.0, 1200.0))
            .build_ui_state(|ui, e: &mut Editor| e.ui(ui), fixture.editor);
        h.run_steps(3);
        let before = h.state().session.project().clone();
        let history = h.state().session.history_len();
        let source = before
            .data()
            .scene()
            .timeline
            .overlays
            .iter()
            .find(|t| t.items.iter().any(|i| i.id == moving))
            .unwrap()
            .id;
        let index = before
            .data()
            .scene()
            .timeline
            .overlays
            .iter()
            .position(|t| t.id == source)
            .unwrap()
            + usize::from(edge == Some(true));
        let body = widget(&h, &format!("timeline.item.{}", moving.0));
        let target_track = if edge.is_some() {
            source
        } else {
            before.data().scene().timeline.main.id
        };
        let header = widget(&h, &format!("track.{}.header", target_track.0));
        let target = egui::pos2(
            body.center().x,
            match edge {
                Some(true) => header.bottom() - 1.0,
                Some(false) => header.top() + 1.0,
                None => header.center().y,
            },
        );
        h.drag_at(body.center());
        h.run_steps(1);
        h.hover_at(target);
        h.run_steps(2);
        let staged = h.state().inspect().unwrap();
        let zone = match edge {
            Some(true) => format!("insert_below:{}", source.0),
            Some(false) => format!("insert_above:{}", source.0),
            None => format!("track:{}", target_track.0),
        };
        assert_eq!(staged.drop_zone.as_deref(), Some(zone.as_str()));
        assert!(staged.preview.is_some(), "the drop zone stages a candidate");
        h.drop_at(target);
        h.run_steps(2);
        let after = h.state().session.project().clone();
        assert_eq!(
            after.data().scene().timeline.track_count(),
            before.data().scene().timeline.track_count() + usize::from(edge.is_some())
        );
        let new = if edge.is_some() {
            &after.data().scene().timeline.overlays[index]
        } else {
            &after.data().scene().timeline.main
        };
        assert_eq!(new.items.len(), 1);
        assert_eq!(new.items[0].id, moving);
        assert_eq!(new.items[0].start, Tick::ZERO);
        assert_eq!(h.state().session.history_len().0, history.0 + 1);
        h.state_mut().session.undo().unwrap();
        assert_eq!(h.state().session.project(), &before);
        h.state_mut().session.redo().unwrap();
        assert_eq!(h.state().session.project(), &after);
        // A video cannot create an overlay inside the audio group; roll back the whole gesture.
        h.run_steps(3);
        let audio = after.data().scene().timeline.audio[0].id;
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

#[test]
fn group_drag_stages_only_the_moved_clips_and_commits_one_step() {
    let fixture = fixture();
    let video_a = fixture.video_a;
    let video_b = fixture.video_b;
    let text = text_item_id(&fixture.editor);
    let audio = audio_item(&fixture.editor);
    let mut editor = fixture.editor;
    editor.selection = [video_b, text].into_iter().collect();
    let committed = editor.session.project().clone();
    let bytes = serde_json::to_vec(committed.data()).unwrap();
    let history = editor.session.history_len();
    // The reference capture's logical content size, so the screenshot can sit
    // beside it at 1:1.
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1727.0, 1029.0))
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor);
    h.run_steps(3);
    let zoom = h.state().inspect().unwrap().zoom;
    let body = widget(&h, &format!("timeline.item.{}", video_b.0));
    let target = egui::pos2(body.center().x + 2.1 * zoom, body.center().y);
    h.drag_at(body.center());
    h.run_steps(1);
    h.hover_at(target);
    h.run_steps(2);
    {
        let state = h.state();
        assert_eq!(
            state.session.project(),
            &committed,
            "the committed project is untouched while previewing"
        );
        assert_eq!(
            serde_json::to_vec(state.session.project().data()).unwrap(),
            bytes
        );
        assert_ne!(state.session.preview_project().unwrap(), &committed);
        let inspection = state.inspect().unwrap();
        assert_eq!(inspection.preview, Some("timeline.move_items"));
        assert_eq!(state.session.history_len(), history);
        assert_eq!(item(state, video_b).start, Tick(8 * SECOND));
        assert_eq!(item(state, text).start, Tick(4 * SECOND));
        assert_eq!(
            item(state, video_b).start - item(state, text).start,
            Tick(4 * SECOND),
            "the group keeps its entry spacing"
        );
        assert_eq!(
            item(state, video_a).start,
            Tick::ZERO,
            "unselected clips do not move"
        );
        assert_eq!(item(state, audio).start, Tick::ZERO);
    }
    let image = h.render().unwrap();
    let target_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-15-drag.png");
    std::fs::create_dir_all(target_path.parent().unwrap()).unwrap();
    image.save(&target_path).unwrap();
    eprintln!("group drag screenshot: {}", target_path.display());
    h.drop_at(target);
    h.run_steps(2);
    assert_eq!(h.state().session.history_len().0, history.0 + 1);
    assert_eq!(item(h.state(), video_b).start, Tick(8 * SECOND));
    assert!(h.state().inspect().unwrap().preview.is_none());
}

#[test]
fn escape_mid_drag_discards_the_staged_candidate() {
    let fixture = fixture();
    let video_b = fixture.video_b;
    let text = text_item_id(&fixture.editor);
    let mut editor = fixture.editor;
    editor.selection = [video_b, text].into_iter().collect();
    let committed = editor.session.project().clone();
    let bytes = serde_json::to_vec(committed.data()).unwrap();
    let history = editor.session.history_len();
    let mut h = harness(editor);
    h.run_steps(3);
    let zoom = h.state().inspect().unwrap().zoom;
    let body = widget(&h, &format!("timeline.item.{}", video_b.0));
    h.drag_at(body.center());
    h.run_steps(1);
    h.hover_at(egui::pos2(body.center().x + 2.0 * zoom, body.center().y));
    h.run_steps(2);
    assert_eq!(item(h.state(), video_b).start, Tick(8 * SECOND));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    let state = h.state();
    assert_eq!(
        serde_json::to_vec(state.session.project().data()).unwrap(),
        bytes,
        "Escape restores the entry project bytes"
    );
    assert_eq!(state.session.project(), &committed);
    assert_eq!(state.session.history_len(), history);
    assert!(!state.session.gesture_active());
    let inspection = state.inspect().unwrap();
    assert!(inspection.preview.is_none() && inspection.timeline_refusal.is_none());
    assert_eq!(item(state, video_b).start, Tick(6 * SECOND));
    assert_eq!(item(state, text).start, Tick(2 * SECOND));
}

#[test]
fn returning_a_group_to_the_press_point_commits_no_history() {
    // Two items on one track so a zero-delta move would otherwise reorder them.
    let mut session = Session::new(
        Project::empty(ProjectId(126), "Reverse".into()).unwrap(),
        20,
    )
    .unwrap();
    let text_track = track(&mut session, "Text", TrackKind::Text);
    let text = ItemParams::Text(project::TextParams {
        content: "Hello".into(),
        font_size: 48.0,
        color: Color([255, 255, 255, 255]),
        ..Default::default()
    });
    let first = insert(
        &mut session,
        text_track,
        Tick::ZERO,
        SourceRange::new(Tick::ZERO, Tick(SECOND)).unwrap(),
        None,
        text.clone(),
    );
    let second = insert(
        &mut session,
        text_track,
        Tick(4 * SECOND),
        SourceRange::new(Tick(4 * SECOND), Tick(5 * SECOND)).unwrap(),
        None,
        text,
    );
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    editor.selection.insert(first);
    let before = serde_json::to_vec(editor.session.project().data()).unwrap();
    let history = editor.session.history_len();
    let mut h = harness(editor);
    h.run_steps(3);
    let body = widget(&h, &format!("timeline.item.{}", first.0));
    let from = body.center();
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(from + egui::vec2(30.0, 0.0));
    h.run_steps(2);
    assert_ne!(item(h.state(), first).start, Tick::ZERO);
    h.hover_at(from);
    h.run_steps(2);
    h.drop_at(from);
    h.run_steps(2);
    assert_eq!(
        serde_json::to_vec(h.state().session.project().data()).unwrap(),
        before,
        "a reversed gesture publishes the entry project"
    );
    assert_eq!(
        h.state().session.history_len(),
        history,
        "a net-zero commit adds no undo step"
    );
    assert_eq!(item(h.state(), first).start, Tick::ZERO);
    assert_eq!(item(h.state(), second).start, Tick(4 * SECOND));
}

#[test]
fn group_trim_previews_every_selected_edge_and_commits_one_step() {
    let fixture = fixture();
    let video_a = fixture.video_a;
    let text = text_item_id(&fixture.editor);
    let mut editor = fixture.editor;
    editor.selection = [video_a, text].into_iter().collect();
    let committed = editor.session.project().clone();
    let history = editor.session.history_len();
    let mut h = harness(editor);
    h.run_steps(3);
    let zoom = h.state().inspect().unwrap().zoom;
    let body_left = widget(&h, "track.0.header").right();
    let handle = widget(&h, &format!("timeline.item.{}.trim_end", video_a.0));
    let target = egui::pos2(body_left + 5.0 * zoom, handle.center().y);
    h.drag_at(handle.center());
    h.run_steps(1);
    h.hover_at(target);
    h.run_steps(2);
    {
        let state = h.state();
        assert_eq!(state.session.project(), &committed);
        assert_eq!(
            state.inspect().unwrap().preview,
            Some("timeline.trim"),
            "resize consumes the group trim command"
        );
        assert_eq!(item(state, video_a).end().unwrap(), Tick(5 * SECOND));
        assert_eq!(
            item(state, text).end().unwrap(),
            Tick(4 * SECOND),
            "every selected end edge moves by the same delta"
        );
        assert_eq!(state.session.history_len(), history);
    }
    h.drop_at(target);
    h.run_steps(2);
    assert_eq!(h.state().session.history_len().0, history.0 + 1);
    assert_eq!(item(h.state(), video_a).end().unwrap(), Tick(5 * SECOND));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(item(h.state(), video_a).end().unwrap(), Tick(6 * SECOND));
    assert_eq!(item(h.state(), text).end().unwrap(), Tick(5 * SECOND));
}

#[test]
fn a_refused_group_move_shows_the_reason_and_discards_the_preview() {
    let fixture = fixture();
    let video_b = fixture.video_b;
    let text = text_item_id(&fixture.editor);
    let text_track = item_track(&fixture.editor, text);
    let mut editor = fixture.editor;
    editor
        .session
        .apply(Command::SetTrackLocked {
            track: text_track,
            locked: true,
        })
        .unwrap();
    editor.selection = [video_b, text].into_iter().collect();
    let before = editor.session.project().clone();
    let history = editor.session.history_len();
    let mut h = harness(editor);
    h.run_steps(3);
    let zoom = h.state().inspect().unwrap().zoom;
    let body = widget(&h, &format!("timeline.item.{}", video_b.0));
    h.drag_at(body.center());
    h.run_steps(1);
    h.hover_at(egui::pos2(body.center().x + 1.0 * zoom, body.center().y));
    h.run_steps(2);
    let state = h.state();
    assert_eq!(
        state.session.project(),
        &before,
        "a refused preview publishes nothing"
    );
    assert_eq!(state.session.history_len(), history);
    assert!(!state.session.gesture_active());
    let inspection = state.inspect().unwrap();
    assert!(inspection.preview.is_none(), "the candidate is discarded");
    let reason = inspection
        .timeline_refusal
        .as_deref()
        .expect("the engine refusal is shown");
    assert!(reason.contains("locked"), "{reason}");
    widget(&h, "timeline.refusal");
    h.drop_at(egui::pos2(body.center().x + 1.0 * zoom, body.center().y));
    h.run_steps(2);
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn a_main_track_item_at_the_origin_is_pinned_and_cannot_move_left() {
    let mut session =
        Session::new(Project::empty(ProjectId(125), "Pinned".into()).unwrap(), 20).unwrap();
    let main = session.project().data().scene().timeline.main.id;
    let item_id = insert(
        &mut session,
        main,
        Tick::ZERO,
        SourceRange::new(Tick::ZERO, Tick(6 * SECOND)).unwrap(),
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
    editor.selection.insert(item_id);
    let history = editor.session.history_len();
    let mut h = harness(editor);
    h.run_steps(3);
    assert!(h.state().inspect().unwrap().pinned.contains(&item_id));
    widget(&h, &format!("timeline.item.{}.pinned", item_id.0));
    let body = widget(&h, &format!("timeline.item.{}", item_id.0));
    drag(
        &mut h,
        body.center(),
        body.center() - egui::vec2(120.0, 0.0),
    );
    assert_eq!(
        item(h.state(), item_id).start,
        Tick::ZERO,
        "the main track's first item stays pinned at the origin"
    );
    assert_eq!(h.state().session.history_len(), history);
}

#[test]
fn zoom_slider_buttons_and_wheel_navigation_persist_through_view_state() {
    let fixture = fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let project_before = serde_json::to_vec(harness.state().session.project().data()).unwrap();
    let before = harness.state().inspect().unwrap();
    assert!(
        before
            .widgets
            .iter()
            .any(|widget| widget.id == "timeline.zoom_slider")
    );
    let zoom_in = widget(&harness, "timeline.zoom_in");
    click_at(&mut harness, zoom_in.center());
    let stepped = harness.state().inspect().unwrap().zoom;
    assert!((stepped - before.zoom * 1.7).abs() < 0.01);
    let zoom_out = widget(&harness, "timeline.zoom_out");
    click_at(&mut harness, zoom_out.center());
    let stepped_back = harness.state().inspect().unwrap().zoom;
    assert!((stepped_back - before.zoom).abs() < 0.01);
    let slider = widget(&harness, "timeline.zoom_slider");
    click_at(
        &mut harness,
        egui::pos2(slider.left() + slider.width() * 0.4, slider.center().y),
    );
    let slider_zoom = harness.state().inspect().unwrap().zoom;
    assert!(slider_zoom > stepped);
    let body = widget(&harness, "timeline.marquee");
    harness.hover_at(body.center());
    harness.run_steps(1);
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 40.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.run_steps(2);
    let wheeled = harness.state().inspect().unwrap().zoom;
    assert!(wheeled > slider_zoom, "ctrl/cmd wheel zooms in");
    let scroll_before = harness.state().inspect().unwrap().scroll;
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -40.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers::SHIFT,
    });
    harness.run_steps(2);
    let after = harness.state().inspect().unwrap();
    assert!(
        after.scroll > scroll_before,
        "shift wheel scrolls horizontally and clamps at the content bound"
    );
    let view = harness.state().session.metadata().view;
    assert!((view.zoom - f64::from(after.zoom) / 40.0).abs() < 1e-6);
    assert!((view.scroll - f64::from(after.scroll * after.zoom)).abs() < 1.0);
    assert_eq!(
        serde_json::to_vec(harness.state().session.project().data()).unwrap(),
        project_before,
        "navigation changes view metadata only, never Project JSON"
    );
}

#[test]
fn edge_auto_scroll_follows_a_dragged_item_and_stops_when_the_pointer_leaves() {
    let mut fixture = fixture();
    fixture
        .editor
        .session
        .apply(Command::SetViewState {
            zoom: 20.0,
            scroll: 4800.0,
            playhead: Tick::ZERO,
        })
        .unwrap();
    let id = fixture.video_b;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let item_rect = widget(&harness, &format!("timeline.item.{}", id.0));
    let body = widget(&harness, "timeline.marquee");
    let start = item(harness.state(), id).start;
    let edge = egui::pos2(body.right() - 8.0, item_rect.center().y);
    harness.drag_at(item_rect.center());
    harness.run_steps(1);
    harness.hover_at(edge);
    harness.run_steps(6);
    let scrolled = harness.state().inspect().unwrap().scroll;
    assert!(scrolled > 0.0, "the right edge advances the viewport");
    assert!(
        item(harness.state(), id).start > start,
        "the dragged item follows the auto-scrolled content"
    );
    harness.hover_at(egui::pos2(body.center().x, body.top() - 4.0));
    harness.run_steps(2);
    let stopped = harness.state().inspect().unwrap().scroll;
    harness.run_steps(3);
    assert_eq!(
        harness.state().inspect().unwrap().scroll,
        stopped,
        "leaving the timeline stops auto-scroll"
    );
    harness.hover_at(edge);
    harness.run_steps(2);
    let gone = harness.state().inspect().unwrap().scroll;
    harness.event(egui::Event::PointerGone);
    harness.run_steps(1);
    assert_eq!(
        harness.state().inspect().unwrap().scroll,
        gone,
        "leaving the window must stop scrolling in the event frame"
    );
    harness.run_steps(3);
    assert_eq!(
        harness.state().inspect().unwrap().scroll,
        gone,
        "a pointer leaving the window must stop auto-scroll despite retained interaction position"
    );
    harness.drop_at(egui::pos2(body.center().x, body.top() - 4.0));
    harness.run_steps(2);
}

#[test]
fn edge_scroll_clamp_reversal_and_escape_preserve_drag_origin() {
    let mut f = fixture();
    f.editor
        .session
        .apply(Command::Delete {
            items: vec![f.video_a],
        })
        .unwrap();
    f.editor.preferences.snapping = false;
    f.editor
        .session
        .apply(Command::SetViewState {
            zoom: 1.0,
            scroll: 0.4,
            playhead: Tick::ZERO,
        })
        .unwrap();
    let id = f.video_b;
    let mut h = harness(f.editor);
    h.run_steps(3);
    let before = h.state().session.project().clone();
    let rect = widget(&h, &format!("timeline.item.{}", id.0));
    let body = widget(&h, "timeline.marquee");
    let press = egui::pos2(rect.left() + 20.0, rect.center().y);
    h.drag_at(press);
    h.run_steps(1);
    let edge = egui::pos2(body.left() + 80.0, press.y);
    h.hover_at(edge);
    h.run_steps(4);
    assert_eq!(h.state().inspect().unwrap().scroll, 0.0);
    let expected = Tick::from_seconds_f64(
        6.0 + f64::from(edge.x - press.x) / 40.0 - 0.01,
        time::Rounding::Nearest,
    )
    .unwrap();
    let actual = item(h.state(), id).start;
    assert!(
        (actual.0 - expected.0).abs() <= SECOND / 30,
        "scroll clamping must compensate only the actual viewport movement: {actual:?} vs {expected:?}"
    );
    h.hover_at(press + egui::vec2(0.4, 0.0));
    h.run_steps(2);
    assert_eq!(item(h.state(), id).start, Tick(6 * SECOND));
    let generation = h.state().playback.generation();
    h.key_press(egui::Key::Escape);
    h.run_steps(1);
    assert_eq!(h.state().session.project(), &before);
    assert!(!h.state().session.gesture_active());
    assert!(h.state().inspect().unwrap().preview.is_none());
    let restored = h.state().playback.generation();
    assert_eq!(
        restored, generation,
        "a staged preview never restarts the player, so Escape needs no new generation"
    );
    // No player request ever saw the drag, so any visible frame belongs to the
    // committed generation rather than to a cancelled candidate.
    if let Some(frame) = h.state().playback.frame() {
        assert!(
            frame.generation <= generation,
            "a cancelled-drag preview frame must not be visible"
        );
    }
}

#[test]
fn timeline_geometry_follows_the_measured_rows() {
    let fixture = fixture();
    let video_a = fixture.video_a;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let ruler = widget(&harness, "timeline.ruler");
    assert_eq!(ruler.height(), 22.0, "ruler strip");
    let clip = widget(&harness, &format!("timeline.item.{}", video_a.0));
    assert_eq!(clip.height(), 74.0, "video clip inside its 76 px lane");
    let header = widget(&harness, "track.1.header");
    assert_eq!((header.width(), header.height()), (123.0, 76.0));
    // Header cells share one 24 px pitch: lock, hide, mute, more.
    let centres: Vec<f32> = ["lock", "hidden", "muted", "more"]
        .into_iter()
        .map(|suffix| widget(&harness, &format!("track.1.{suffix}")).center().x)
        .collect();
    for pair in centres.windows(2) {
        assert!((pair[1] - pair[0] - 24.0).abs() < 0.5, "pitch {centres:?}");
    }
    // Toolbar buttons keep a 36 px pitch (24 px box, 12 px gap).
    let undo = widget(&harness, "edit.undo");
    let redo = widget(&harness, "edit.redo");
    assert!((redo.center().x - undo.center().x - 36.0).abs() < 0.5);
    let slider = widget(&harness, "timeline.zoom_slider");
    assert!(
        painted_shapes(&harness).into_iter().any(|shape| {
            matches!(shape, egui::Shape::Rect(rect)
            if rect.rect.size() == egui::vec2(8.0, 12.0)
                && slider.contains_rect(rect.rect))
        }),
        "zoom thumb follows the measured 8 by 12 px owner value"
    );
}

#[test]
fn horizontal_scrollbar_has_a_visible_thumb() {
    let mut h = harness(fixture().editor);
    h.run_steps(3);
    let bar = widget(&h, "timeline.scroll_h");
    assert_eq!(bar.height(), 6.0);
    assert!(
        painted_shapes(&h).into_iter().any(|shape| {
            matches!(shape, egui::Shape::Rect(rect)
            if rect.rect.height() == 6.0 && rect.rect.width() > 0.0
                && rect.rect.width() < bar.width()
                && bar.contains_rect(rect.rect) && rect.fill.a() > 0)
        }),
        "the scrollbar must paint a nonzero thumb inside its hit area"
    );
}

#[test]
fn clip_details_remain_visible_after_scrolling_deep_into_a_long_clip() {
    let mut f = fixture();
    let duration = Tick(60 * SECOND);
    let source = SourceAsset::try_from(AssetData {
        ephemeral: false,
        id: AssetId(3),
        path: std::env::temp_dir().join("vibecut-long-timeline.mp4"),
        content_hash: ContentHash::from_bytes(&[3; 8]),
        probe: SourceProbe {
            format: "mov,mp4".into(),
            duration: Some(duration),
            streams: vec![source_stream(media::StreamKind::Video, duration)],
        },
    })
    .unwrap();
    f.editor
        .session
        .apply(Command::AddAsset { asset: source })
        .unwrap();
    let video = track(&mut f.editor.session, "Long video", TrackKind::Video);
    let id = insert(
        &mut f.editor.session,
        video,
        Tick::ZERO,
        SourceRange::new(Tick::ZERO, duration).unwrap(),
        Some(AssetId(3)),
        ItemParams::Video(project::VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
        }),
    );
    let mut h = harness(f.editor);
    for scroll in [0.0, 4_000.0, 40_000.0] {
        h.state_mut()
            .session
            .apply(Command::SetViewState {
                zoom: 100.0,
                scroll,
                playhead: Tick::ZERO,
            })
            .unwrap();
        h.run_steps(3);
        let clip = widget(&h, &format!("timeline.item.{}", id.0));
        let middle = clip.center().x;
        let shapes = painted_shapes(&h);
        assert!(
            shapes.iter().any(|shape| {
                matches!(shape, egui::Shape::Rect(rect)
                if rect.rect.height() > 37.0 && rect.rect.height() < 40.0
                    && rect.rect.contains(egui::pos2(middle, clip.top() + 30.0)))
            }),
            "filmstrip must cover the visible clip at scroll {scroll}"
        );
        assert!(
            shapes.iter().any(|shape| {
                matches!(shape, egui::Shape::LineSegment { points, .. }
                if points[0].x.min(points[1].x) <= middle
                    && points[0].x.max(points[1].x) >= middle
                    && points[0].y > clip.bottom() - 17.0
                    && points[0].y < clip.bottom()
                    && points[1].y > clip.bottom() - 17.0
                    && points[1].y < clip.bottom())
            }),
            "waveform must cover the visible clip at scroll {scroll}"
        );
    }
}

fn painted_shapes<'a>(harness: &'a Harness<'_, Editor>) -> Vec<&'a egui::Shape> {
    let mut pending: Vec<_> = harness
        .output()
        .shapes
        .iter()
        .map(|shape| &shape.shape)
        .collect();
    let mut shapes = Vec::new();
    while let Some(shape) = pending.pop() {
        if let egui::Shape::Vec(children) = shape {
            pending.extend(children);
        } else {
            shapes.push(shape);
        }
    }
    shapes
}

#[test]
fn header_lock_control_toggles_the_track_lock() {
    let fixture = fixture();
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let locked = |editor: &Editor| {
        editor
            .session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .iter()
            .find(|track| track.id.0 == 1)
            .unwrap()
            .locked
    };
    assert!(!locked(harness.state()));
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("track.1.lock")))
        .click();
    harness.run_steps(3);
    assert!(locked(harness.state()), "lock control locks the track");
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("track.1.lock")))
        .click();
    harness.run_steps(3);
    assert!(!locked(harness.state()), "second press unlocks it");
}

#[test]
fn property_diamond_toggles_a_keyframe_at_the_playhead() {
    let fixture = fixture();
    let id = fixture.video_a;
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    harness
        .get(
            By::new()
                .predicate(|n| n.author_id() == Some(format!("timeline.item.{}", id.0).as_str())),
        )
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.group.basic")))
        .click();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.tab.blending")))
        .click();
    harness.run_steps(2);
    harness
        .state_mut()
        .dispatch(AppCommand::Seek(Tick(2 * SECOND)))
        .unwrap();
    harness.run_steps(2);
    let opacity = project::AnimationPath::new("opacity").unwrap();
    let key_count = |harness: &Harness<'_, Editor>| {
        item(harness.state(), id)
            .animations
            .get(&opacity)
            .map_or(0, |param| match param {
                project::Param::Value(channel) => channel.keys().len(),
                project::Param::Color(_) => 0,
            })
    };
    let diamond = widget(&harness, "inspector.keyframe.opacity");
    click_at(&mut harness, diamond.center());
    assert_eq!(key_count(&harness), 1);
    // Editing the animated field writes a key at the playhead instead of wiping curves.
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.opacity")))
        .click();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("50".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(key_count(&harness), 1);
    assert_eq!(
        item(harness.state(), id)
            .animations
            .get(&opacity)
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()[0]
            .value,
        project::ParamValue::Scalar(0.5)
    );
    assert_eq!(
        harness
            .state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|candidate| candidate.id == id)
            .unwrap()
            .animations
            .get(&opacity)
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()[0]
            .time,
        Tick(2 * SECOND)
    );
    let diamond = widget(&harness, "inspector.keyframe.opacity");
    click_at(&mut harness, diamond.center());
    assert_eq!(key_count(&harness), 0, "a second press removes the key");
    // An X/Y pair shares one diamond: both keys land, and go, in one undo step.
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("inspector.tab.transform")))
        .click();
    harness.run_steps(2);
    let keys_of = |harness: &Harness<'_, Editor>, field: &str| {
        let path = project::AnimationPath::new(field).unwrap();
        item(harness.state(), id)
            .animations
            .get(&path)
            .map_or(0, |param| match param {
                project::Param::Value(channel) => channel.keys().len(),
                project::Param::Color(_) => 0,
            })
    };
    let history = harness.state().session.history_len();
    let diamond = widget(&harness, "inspector.keyframe.transform.positionX");
    click_at(&mut harness, diamond.center());
    assert_eq!(keys_of(&harness, "transform.positionX"), 1);
    assert_eq!(keys_of(&harness, "transform.positionY"), 1);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    let diamond = widget(&harness, "inspector.keyframe.transform.positionX");
    click_at(&mut harness, diamond.center());
    assert_eq!(keys_of(&harness, "transform.positionX"), 0);
    assert_eq!(keys_of(&harness, "transform.positionY"), 0);
    assert_eq!(harness.state().session.history_len().0, history.0 + 2);
}

#[test]
fn text_fade_keys_open_in_the_ordinary_lane_and_delete_with_undo() {
    let mut e = fixture().editor;
    e.activate("text.insert").unwrap();
    let id = *e.selection.first().unwrap();
    e.activate("text.animation.fade_in_out").unwrap();
    let mut h = harness(e);
    h.run_steps(3);
    let lanes = widget(&h, &format!("timeline.item.{}.lanes", id.0));
    click_at(&mut h, lanes.center());
    let key = widget(
        &h,
        &format!("timeline.keyframe.{}.opacity.none.76204800000", id.0),
    );
    click_at(&mut h, key.center());
    let before = h.state().session.project().clone();
    h.state_mut().activate("timeline.delete").unwrap();
    h.run_steps(2);
    let opacity = project::BuiltinPath::Opacity.into();
    assert_eq!(
        item(h.state(), id)
            .animations
            .get(&opacity)
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .len(),
        3
    );
    h.state_mut().session.undo().unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn expanded_lane_shows_diamonds_and_drag_retimes_one_undo_step() {
    let fixture = fixture();
    let id = fixture.video_a;
    let mut editor = fixture.editor;
    add_key(&mut editor, id, "opacity", Tick(2 * SECOND), 0.4);
    add_key(&mut editor, id, "opacity", Tick(4 * SECOND), 0.8);
    let mut harness = harness(editor);
    harness.run_steps(3);
    let lanes = widget(&harness, &format!("timeline.item.{}.lanes", id.0));
    click_at(&mut harness, lanes.center());
    assert_eq!(harness.state().inspect().unwrap().expanded_lanes, vec![id]);
    let dir = tempfile::tempdir().unwrap();
    let store = vibecut::preferences::PreferenceStore::new(dir.path().join("preferences.json"));
    store.save(&harness.state().preferences).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(
        loaded.timeline_lanes.get(&ProjectId(12)).cloned(),
        Some(std::collections::BTreeSet::from([id.0])),
        "expanded lanes round-trip through the preference store"
    );
    // A reopen restores the owning project's lane; another project is not
    // affected even though its item ids restart at the same values.
    let data = harness.state().session.project().data().clone();
    let reopened = editor_for(Project::new(data.clone()).unwrap(), loaded.clone());
    assert_eq!(reopened.inspect().unwrap().expanded_lanes, vec![id]);
    let mut other = data;
    other.id = ProjectId(13);
    other.name = "Other".into();
    let mut other_harness =
        crate::timeline_ui::harness(editor_for(Project::new(other).unwrap(), loaded));
    other_harness.run_steps(3);
    assert!(
        other_harness
            .state()
            .inspect()
            .unwrap()
            .expanded_lanes
            .is_empty(),
        "another project starts with no open lanes"
    );
    let other_lanes = widget(&other_harness, &format!("timeline.item.{}.lanes", id.0));
    click_at(&mut other_harness, other_lanes.center());
    store.save(&other_harness.state().preferences).unwrap();
    let stored = store.load().unwrap();
    assert_eq!(
        stored.timeline_lanes.get(&ProjectId(12)).cloned(),
        Some(std::collections::BTreeSet::from([id.0])),
        "toggling a lane in another project keeps this project's lanes"
    );
    assert_eq!(
        stored.timeline_lanes.get(&ProjectId(13)).cloned(),
        Some(std::collections::BTreeSet::from([id.0])),
        "the other project stores its own open lanes"
    );
    let other_lanes = widget(&other_harness, &format!("timeline.item.{}.lanes", id.0));
    click_at(&mut other_harness, other_lanes.center());
    store.save(&other_harness.state().preferences).unwrap();
    let collapsed = store.load().unwrap();
    assert_eq!(
        collapsed.timeline_lanes.get(&ProjectId(12)).cloned(),
        Some(std::collections::BTreeSet::from([id.0])),
        "collapsing every lane in another project keeps this project's lanes"
    );
    assert!(
        !collapsed.timeline_lanes.contains_key(&ProjectId(13)),
        "a project with no open lanes keeps no empty entry"
    );
    // Save As carries the issuing project's lane set to the fresh ProjectId;
    // the original keeps its own set and an already-empty source carries nothing.
    let mut carried = collapsed.clone();
    carried.copy_timeline_lanes(ProjectId(12), ProjectId(14));
    assert_eq!(
        carried.timeline_lanes.get(&ProjectId(12)).cloned(),
        Some(std::collections::BTreeSet::from([id.0])),
        "Save As leaves the issuing project's lanes in place"
    );
    assert_eq!(
        carried.timeline_lanes.get(&ProjectId(14)).cloned(),
        Some(std::collections::BTreeSet::from([id.0])),
        "the copy opens with the issuing project's lanes"
    );
    let mut absent = collapsed;
    absent.copy_timeline_lanes(ProjectId(99), ProjectId(14));
    assert!(
        !absent.timeline_lanes.contains_key(&ProjectId(14)),
        "a project with no open lanes carries no entry"
    );
    drop(other_harness);
    let key = format!("timeline.keyframe.{}.opacity.none.{}", id.0, 2 * SECOND);
    let diamond = widget(&harness, &key);
    let history = harness.state().session.history_len().0;
    let zoom = harness.state().inspect().unwrap().zoom;
    drag(
        &mut harness,
        diamond.center(),
        egui::pos2(diamond.center().x + zoom, diamond.center().y),
    );
    let channel = item(harness.state(), id)
        .animations
        .get(&project::AnimationPath::new("opacity").unwrap())
        .unwrap()
        .channel(None)
        .unwrap();
    assert_eq!(channel.keys()[0].time, Tick(3 * SECOND));
    assert_eq!(harness.state().session.history_len().0, history + 1);
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(2);
    let channel = item(harness.state(), id)
        .animations
        .get(&project::AnimationPath::new("opacity").unwrap())
        .unwrap()
        .channel(None)
        .unwrap();
    assert_eq!(channel.keys()[0].time, Tick(2 * SECOND));

    // A keyframe drag snaps to the playhead like a clip edge does.
    harness
        .state_mut()
        .dispatch(AppCommand::Seek(Tick(3 * SECOND)))
        .unwrap();
    harness.run_steps(2);
    let key = widget(
        &harness,
        &format!("timeline.keyframe.{}.opacity.none.{}", id.0, 2 * SECOND),
    );
    drag(
        &mut harness,
        key.center(),
        egui::pos2(key.center().x + zoom * 1.1, key.center().y),
    );
    let channel = item(harness.state(), id)
        .animations
        .get(&project::AnimationPath::new("opacity").unwrap())
        .unwrap()
        .channel(None)
        .unwrap();
    assert_eq!(channel.keys()[0].time, Tick(3 * SECOND), "playhead snap");
}

#[test]
fn lane_marquee_selects_keys_and_delete_removes_them_atomically() {
    let fixture = fixture();
    let id = fixture.video_a;
    let mut editor = fixture.editor;
    add_key(&mut editor, id, "opacity", Tick(2 * SECOND), 0.4);
    add_key(&mut editor, id, "opacity", Tick(4 * SECOND), 0.8);
    let mut harness = harness(editor);
    harness.run_steps(3);
    let lanes = widget(&harness, &format!("timeline.item.{}.lanes", id.0));
    click_at(&mut harness, lanes.center());
    let first = widget(
        &harness,
        &format!("timeline.keyframe.{}.opacity.none.{}", id.0, 2 * SECOND),
    );
    let second = widget(
        &harness,
        &format!("timeline.keyframe.{}.opacity.none.{}", id.0, 4 * SECOND),
    );
    drag(
        &mut harness,
        egui::pos2(first.left() - 12.0, first.center().y),
        egui::pos2(second.right() + 12.0, second.center().y),
    );
    assert_eq!(
        harness.state().inspect().unwrap().selection_scope,
        vibecut::selection::SelectionScope::Keyframes
    );
    let history = harness.state().session.history_len().0;
    harness.key_press(egui::Key::Delete);
    harness.run_steps(3);
    let channel = item(harness.state(), id)
        .animations
        .get(&project::AnimationPath::new("opacity").unwrap())
        .unwrap()
        .channel(None)
        .unwrap();
    assert!(channel.keys().is_empty());
    assert_eq!(harness.state().session.history_len().0, history + 1);
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(2);
    let channel = item(harness.state(), id)
        .animations
        .get(&project::AnimationPath::new("opacity").unwrap())
        .unwrap()
        .channel(None)
        .unwrap();
    assert_eq!(channel.keys().len(), 2);
}

#[test]
fn volume_line_drag_commits_a_tenth_db_step_and_keys_survive_reload() {
    let fixture = fixture();
    let audio = audio_item(&fixture.editor);
    let mut harness = harness(fixture.editor);
    harness.run_steps(3);
    let line = widget(&harness, &format!("timeline.volume.{}", audio.0));
    let history = harness.state().session.history_len().0;
    drag(
        &mut harness,
        line.center(),
        egui::pos2(line.center().x, line.center().y - line.height() * 0.2),
    );
    let ItemParams::Audio(params) = &item(harness.state(), audio).content else {
        panic!("audio")
    };
    assert!(
        (params.volume * 10.0 - (params.volume * 10.0).round()).abs() < 1e-3,
        "drag commits a 0.1 dB step, got {}",
        params.volume
    );
    assert!(params.volume > 0.0);
    assert_eq!(harness.state().session.history_len().0, history + 1);
    let volume = params.volume;
    add_key(
        harness.state_mut(),
        audio,
        "volume",
        Tick(3 * SECOND),
        f64::from(volume),
    );
    let encoded = project::encode(harness.state().session.project(), false).unwrap();
    let decoded = project::decode(&encoded).unwrap();
    let channel = decoded
        .project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == audio)
        .unwrap()
        .animations
        .get(&project::AnimationPath::new("volume").unwrap())
        .unwrap()
        .channel(None)
        .unwrap();
    assert_eq!(channel.keys().len(), 1);
    assert_eq!(channel.keys()[0].time, Tick(3 * SECOND));
}

#[test]
fn keyframe_lanes_and_volume_line_render_the_audit_screenshot() {
    let fixture = fixture();
    let id = fixture.video_a;
    let audio = audio_item(&fixture.editor);
    let mut editor = fixture.editor;
    add_key(&mut editor, id, "opacity", Tick(2 * SECOND), 0.35);
    add_key(&mut editor, id, "opacity", Tick(4 * SECOND), 0.9);
    add_key(&mut editor, audio, "volume", Tick(2 * SECOND), 0.0);
    add_key(&mut editor, audio, "volume", Tick(7 * SECOND), -12.0);
    let mut harness = harness(editor);
    harness.run_steps(3);
    let lanes = widget(&harness, &format!("timeline.item.{}.lanes", id.0));
    click_at(&mut harness, lanes.center());
    let audio_lanes = widget(&harness, &format!("timeline.item.{}.lanes", audio.0));
    click_at(&mut harness, audio_lanes.center());
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .expanded_lanes
            .contains(&audio),
        "audio lanes open"
    );
    let _ = widget(&harness, &format!("timeline.lane.{}.volume", audio.0));
    let image = harness.render().unwrap();
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    let path = target.join("p1-39-keyframes.png");
    image.save(&path).unwrap();
    eprintln!("keyframe screenshot: {}", path.display());
    assert!(path.is_file());
}

#[test]
fn graph_popover_stages_curves_and_commits_or_cancels_in_one_undo_step() {
    let fixture = fixture();
    let id = fixture.video_a;
    let mut editor = fixture.editor;
    add_key(&mut editor, id, "opacity", Tick(2 * SECOND), 0.2);
    add_key(&mut editor, id, "opacity", Tick(4 * SECOND), 0.9);
    let mut harness = harness(editor);
    harness.run_steps(3);
    let by =
        |author: &'static str| By::new().predicate(move |node| node.author_id() == Some(author));
    let curve_of = |harness: &Harness<'_, Editor>| {
        item(harness.state(), id)
            .animations
            .get(&project::AnimationPath::new("opacity").unwrap())
            .unwrap()
            .channel(None)
            .unwrap()
            .segment_curve(Tick(2 * SECOND))
            .unwrap()
            .0
    };
    let select = |harness: &mut Harness<'_, Editor>| {
        harness.state_mut().selection_scopes.keyframes.insert(
            vibecut::selection::KeyframeSelection {
                item: id,
                path: project::AnimationPath::new("opacity").unwrap(),
                component: None,
                time: Tick(2 * SECOND),
            },
        );
        harness.run_steps(3);
    };
    let bytes = |harness: &Harness<'_, Editor>| {
        project::encode(harness.state().session.project(), false).unwrap()
    };
    assert!(
        harness
            .get(by("timeline.graph"))
            .accesskit_node()
            .is_disabled(),
        "no keyframe is selected"
    );
    select(&mut harness);
    assert!(
        !harness
            .get(by("timeline.graph"))
            .accesskit_node()
            .is_disabled()
    );

    // Opening and previewing never mutate the committed project or its history.
    let start = bytes(&harness);
    let history = harness.state().session.history_len().0;
    harness.get(by("timeline.graph")).click();
    harness.run_steps(3);
    assert_eq!(
        curve_of(&harness),
        [0.0, 0.0, 1.0, 1.0],
        "a linear segment opens as the line"
    );
    harness.get(by("timeline.graph.preset.pop")).click();
    harness.run_steps(3);
    // Source table value for Pop (easing-presets.ts line 17), within whole-tick rounding.
    for (got, want) in curve_of(&harness)
        .into_iter()
        .zip([0.175, 0.885, 0.32, 1.275])
    {
        assert!((got - want).abs() < 1e-3, "{got} != {want}");
    }
    // An explicit gesture exposes its candidate through `project()`; only the
    // history and the Cancel restore below prove nothing was committed.
    assert_ne!(bytes(&harness), start, "the staged preset is the candidate");
    assert_eq!(harness.state().session.history_len().0, history);

    // Dragging the outgoing handle updates the preview curve live.
    let plot = widget(&harness, "timeline.graph.plot").shrink(14.0);
    let at = |x: f64, y: f64| {
        egui::pos2(
            plot.left() + x as f32 * plot.width(),
            plot.bottom() - ((y + 0.5) / 2.0) as f32 * plot.height(),
        )
    };
    drag(&mut harness, at(0.175, 0.885), at(0.5, 1.0));
    let dragged = curve_of(&harness);
    assert!(
        (dragged[0] - 0.5).abs() < 0.02 && (dragged[1] - 1.0).abs() < 0.02,
        "{dragged:?}"
    );
    assert!((dragged[2] - 0.32).abs() < 1e-3, "the other handle stays");
    assert_eq!(harness.state().session.history_len().0, history);

    // Apply publishes everything staged as one undo step.
    harness.get(by("timeline.graph.apply")).click();
    harness.run_steps(3);
    assert_eq!(harness.state().session.history_len().0, history + 1);
    let applied = bytes(&harness);
    assert_ne!(applied, start);
    assert!(
        harness.query(by("timeline.graph.plot")).is_none(),
        "Apply closes the popover"
    );
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(2);
    assert_eq!(bytes(&harness), start);
    harness.state_mut().session.redo().unwrap();
    harness.run_steps(2);
    assert_eq!(bytes(&harness), applied);
    let applied_curve = curve_of(&harness);

    // Cancel and Escape restore the pre-open state exactly, with no history.
    for escape in [false, true] {
        select(&mut harness);
        harness.get(by("timeline.graph")).click();
        harness.run_steps(3);
        harness.get(by("timeline.graph.preset.ease_in")).click();
        harness.run_steps(3);
        assert_ne!(curve_of(&harness), applied_curve, "the preset previews");
        if escape {
            harness.key_press(egui::Key::Escape);
        } else {
            harness.get(by("timeline.graph.cancel")).click();
        }
        harness.run_steps(3);
        assert!(harness.query(by("timeline.graph.plot")).is_none());
        assert_eq!(bytes(&harness), applied, "cancel restores the project");
        assert_eq!(curve_of(&harness), applied_curve);
        assert_eq!(harness.state().session.history_len().0, history + 1);
        assert!(harness.state().session.preview_project().is_none());
    }

    // A saved curve persists with the popover tab and applies from the Saved tab.
    select(&mut harness);
    harness.get(by("timeline.graph")).click();
    harness.run_steps(3);
    harness.get(by("timeline.graph.save")).click();
    harness.run_steps(3);
    harness.get(by("timeline.graph.tab.saved")).click();
    harness.run_steps(3);
    let easing = harness.state().preferences.easing.clone();
    assert_eq!(easing.tab, vibecut::preferences::EasingTab::Saved);
    assert_eq!(easing.saved.len(), 1);
    assert_eq!(easing.saved[0].name, "Custom 1");
    let mut restored = vibecut::preferences::Preferences::default();
    restored
        .set("easing", harness.state().preferences.get("easing").unwrap())
        .unwrap();
    assert_eq!(restored.easing, easing, "saved presets and tab round-trip");
    harness.get(by("timeline.graph.saved.0")).click();
    harness.run_steps(3);
    assert_eq!(
        curve_of(&harness),
        applied_curve,
        "the saved curve is the applied one"
    );
    let image = harness.render().unwrap();
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    let path = target.join("p1-40-graph.png");
    image.save(&path).unwrap();
    eprintln!("graph screenshot: {}", path.display());
    assert!(path.is_file());
    harness.get(by("timeline.graph.cancel")).click();
    harness.run_steps(3);
    assert_eq!(bytes(&harness), applied);
}

#[test]
fn ripple_toolbar_delete_trim_split_and_cross_track_move_share_the_policy() {
    for enabled in [false, true] {
        let fixture = fixture();
        let a = fixture.video_a;
        let b = fixture.video_b;
        let mut harness = crate::gpu::builder()
            .with_size(egui::vec2(1727.0, 1029.0))
            .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
        harness.run_steps(3);
        if enabled {
            harness
                .get(By::new().predicate(|n| n.author_id() == Some("timeline.ripple")))
                .click();
            harness.run_steps(2);
        }
        assert_eq!(harness.state().preferences.ripple, enabled);
        let before = project::encode(harness.state().session.project(), false).unwrap();
        let history = harness.state().session.history_len().0;
        let zoom = harness.state().inspect().unwrap().zoom;
        let handle = widget(&harness, &format!("timeline.item.{}.trim_end", a.0));
        if enabled {
            // A return to the press point restores the follower as well as the edge.
            harness.drag_at(handle.center());
            harness.run_steps(1);
            harness.hover_at(handle.center() - egui::vec2(zoom, 0.0));
            harness.run_steps(2);
            assert_eq!(item(harness.state(), b).start, Tick(5 * SECOND));
            assert_eq!(
                project::encode(harness.state().session.project(), false).unwrap(),
                before
            );
            let generation = harness.state().playback.generation();
            harness.hover_at(handle.center());
            harness.run_steps(2);
            assert!(
                harness.state().playback.generation() > generation,
                "reversal must refresh the restored preview"
            );
            harness.drop_at(handle.center());
            harness.run_steps(2);
            assert_eq!(
                project::encode(harness.state().session.project(), false).unwrap(),
                before
            );
            assert_eq!(harness.state().session.history_len().0, history);
        }
        drag(
            &mut harness,
            handle.center(),
            handle.center() - egui::vec2(zoom, 0.0),
        );
        assert_eq!(item(harness.state(), a).end().unwrap(), Tick(5 * SECOND));
        assert_eq!(
            item(harness.state(), b).start,
            Tick(if enabled { 5 } else { 6 } * SECOND)
        );
        assert_eq!(harness.state().session.history_len().0, history + 1);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.run_steps(2);
        assert_eq!(
            project::encode(harness.state().session.project(), false).unwrap(),
            before
        );
        // A split covers the complete former span and cannot ripple its follower.
        let snapshot = harness.state().session.project().clone();
        harness
            .state_mut()
            .playback
            .seek(Tick(3 * SECOND), false, snapshot)
            .unwrap();
        harness.state_mut().selection = [a].into_iter().collect();
        let before_split = count(harness.state());
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
        harness.run_steps(2);
        assert_eq!(
            count(harness.state()),
            before_split + 1,
            "the split must actually run"
        );
        assert_eq!(item(harness.state(), b).start, Tick(6 * SECOND));
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.run_steps(2);
        let target = track(
            &mut harness.state_mut().session,
            "Overlay",
            TrackKind::Video,
        );
        harness.run_steps(2);
        let body = widget(&harness, &format!("timeline.item.{}", a.0));
        let destination = widget(&harness, &format!("track.{}.header", target.0));
        drag(
            &mut harness,
            body.center(),
            egui::pos2(body.center().x, destination.center().y),
        );
        let location = harness
            .state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .find(|t| t.items.iter().any(|i| i.id == a))
            .unwrap()
            .id;
        assert_eq!(location, target);
        assert_eq!(item(harness.state(), b).start, Tick(6 * SECOND));
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.run_steps(2);
        harness.state_mut().selection = [a].into_iter().collect();
        harness.key_press(egui::Key::Delete);
        harness.run_steps(2);
        assert!(item_opt(harness.state(), a).is_none());
        assert_eq!(
            item(harness.state(), b).start,
            Tick(if enabled { 0 } else { 6 } * SECOND)
        );
        {
            eprintln!(
                "ripple toggle enabled={enabled}: {:?}",
                widget(&harness, "timeline.ripple")
            );
            let image = harness.render().unwrap();
            let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
            std::fs::create_dir_all(&target).unwrap();
            image
                .save(target.join(if enabled {
                    "p1-22-ripple.png"
                } else {
                    "p1-22-ripple-off.png"
                }))
                .unwrap();
        }
    }
}

/// The shared timeline fixture with audible source video, so extraction can run.
fn audible_editor() -> Fixture {
    let mut fixture = fixture();
    let mut data = fixture.editor.session.project().data().clone();
    let position = data
        .assets
        .iter()
        .position(|asset| asset.id() == AssetId(1))
        .unwrap();
    data.assets[position] =
        testkit::source::asset(1, std::env::temp_dir().join("vibecut-timeline-1.mp4"))
            .format("mov,mp4")
            .duration(Tick(10 * SECOND))
            .content_hash(ContentHash::from_bytes(&[1; 8]))
            .stream(
                testkit::source::video()
                    .time_base(media::TimeBase::new(1, 30000).unwrap())
                    .duration(Tick(10 * SECOND))
                    .start(Tick::ZERO)
                    .size(64, 64)
                    .build(),
            )
            .stream(
                testkit::source::audio()
                    .index(1)
                    .time_base(media::TimeBase::new(1, 48000).unwrap())
                    .duration(Tick(10 * SECOND))
                    .start(Tick::ZERO)
                    .build(),
            )
            .build();
    let session = Session::new(Project::new(data).unwrap(), 50).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    fixture.editor = Editor::new(session, playback, Preferences::default());
    fixture
}

fn click_item(harness: &mut Harness<'_, Editor>, id: ItemId) {
    harness
        .get(By::new().predicate(move |node| {
            node.author_id() == Some(format!("timeline.item.{}", id.0).as_str())
        }))
        .click();
    harness.run_steps(2);
}

fn open_item_menu(harness: &mut Harness<'_, Editor>, id: ItemId, suffix: &'static str) {
    harness
        .get(By::new().predicate(move |node| {
            node.author_id() == Some(format!("timeline.item.{}", id.0).as_str())
        }))
        .click_secondary();
    harness.run_steps(2);
    harness
        .get(By::new().predicate(move |node| {
            node.author_id() == Some(format!("timeline.item.{}.{suffix}", id.0).as_str())
        }))
        .click();
    harness.run_steps(3);
}

fn menu_enabled(harness: &Harness<'_, Editor>, id: &str) -> bool {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == id)
        .unwrap_or_else(|| panic!("missing menu widget {id}"))
        .enabled
}

fn open_author_menu(harness: &mut Harness<'_, Editor>, author: String) {
    harness
        .get(By::new().predicate(move |node| node.author_id() == Some(author.as_str())))
        .click_secondary();
    harness.run_steps(3);
}

fn close_menu(harness: &mut Harness<'_, Editor>) {
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
}

#[test]
fn context_and_header_menus_expose_registered_items_and_reasons() {
    let fixture = audible_editor();
    let video_a = fixture.video_a;
    let mut h = harness(fixture.editor);
    h.run_steps(3);
    h.state_mut()
        .dispatch(AppCommand::Seek(Tick(2 * SECOND)))
        .unwrap();
    h.run_steps(2);
    click_item(&mut h, video_a);
    let before = project::encode(h.state().session.project(), false).unwrap();

    open_author_menu(&mut h, format!("timeline.item.{}", video_a.0));
    for id in [
        "split",
        "split_left",
        "split_right",
        "duplicate",
        "delete",
        "hide",
        "mute",
        "audio",
        "reveal",
    ] {
        assert!(
            menu_enabled(&h, &format!("timeline.item.{}.{id}", video_a.0)),
            "clip menu {id}"
        );
    }
    assert!(
        !menu_enabled(&h, &format!("timeline.item.{}.paste", video_a.0)),
        "the empty clipboard disables paste"
    );
    let image = h.render().unwrap();
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-07-menus.png")).unwrap();
    close_menu(&mut h);

    let track = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| track.name.as_str() == "Video")
        .unwrap()
        .id;
    h.get(By::new().predicate(move |node| {
        node.author_id() == Some(format!("track.{}.more", track.0).as_str())
    }))
    .click();
    h.run_steps(3);
    for (suffix, enabled) in [
        ("rename_action", true),
        ("muted", true),
        ("hidden", true),
        ("delete", true),
        ("paste", false),
    ] {
        assert_eq!(
            menu_enabled(&h, &format!("track.{}.{suffix}", track.0)),
            enabled,
            "track menu {suffix}"
        );
    }
    close_menu(&mut h);

    open_author_menu(&mut h, "preview.canvas".to_owned());
    for (id, enabled) in [
        ("preview.menu.snapshot", true),
        ("preview.menu.copy", true),
        ("preview.menu.fullscreen", true),
        ("preview.menu.jump.next", true),
        ("preview.menu.jump.previous", false),
    ] {
        assert_eq!(menu_enabled(&h, id), enabled, "preview menu {id}");
    }
    close_menu(&mut h);

    h.get(By::new().predicate(|node| node.author_id() == Some("media.asset.1")))
        .click_secondary();
    h.run_steps(3);
    assert!(menu_enabled(&h, "media.delete"));
    assert!(
        !menu_enabled(&h, "media.export_clip"),
        "clip export is unavailable until its engine command lands"
    );
    close_menu(&mut h);

    h.get(By::new().predicate(|node| node.author_id() == Some("window.menu")))
        .click();
    h.run_steps(2);
    for id in [
        "project.exit",
        "shortcuts.open",
        "app.help",
        "window.home",
        "project.save",
        "project.save_as",
        "preferences.open",
        "models.open",
        "layout.reset",
        "appearance.locale",
        "appearance.theme",
    ] {
        assert!(menu_enabled(&h, id), "header menu {id}");
    }
    close_menu(&mut h);
    assert!(menu_enabled(&h, "app.feedback"));
    let freeze = h
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == "timeline.freeze")
        .expect("freeze toolbar cell");
    assert!(!freeze.enabled);
    assert!(
        freeze
            .tooltip
            .as_ref()
            .is_some_and(|tooltip| tooltip.contains("Not available yet")),
        "{:?}",
        freeze.tooltip
    );
    assert_eq!(
        project::encode(h.state().session.project(), false).unwrap(),
        before,
        "opening menus changes no project bytes"
    );
}

#[test]
fn clip_action_keys_and_context_menu_apply_each_command_with_undo() {
    let fixture = audible_editor();
    let video_a = fixture.video_a;
    let video_b = fixture.video_b;
    let audio = audio_item(&fixture.editor);
    let mut h = harness(fixture.editor);
    h.run_steps(3);
    h.state_mut()
        .dispatch(AppCommand::Seek(Tick(2 * SECOND)))
        .unwrap();
    h.run_steps(2);
    let undo = |h: &mut Harness<'_, Editor>| {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
    };
    let redo = |h: &mut Harness<'_, Editor>| {
        h.key_press_modifiers(
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
            egui::Key::Z,
        );
        h.run_steps(2);
    };
    let seek = |h: &mut Harness<'_, Editor>| {
        h.state_mut()
            .dispatch(AppCommand::Seek(Tick(2 * SECOND)))
            .unwrap();
        h.run_steps(1);
    };

    click_item(&mut h, video_a);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    h.run_steps(2);
    assert_eq!(count(h.state()), 5, "Ctrl+B splits the selected item");
    undo(&mut h);
    assert_eq!(count(h.state()), 4);

    // With an empty selection every unlocked item under the playhead splits.
    h.state_mut().activate("timeline.deselect_all").unwrap();
    seek(&mut h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    h.run_steps(2);
    assert_eq!(
        count(h.state()),
        6,
        "empty selection splits the video and audio clips under the playhead"
    );
    undo(&mut h);
    assert_eq!(count(h.state()), 4);

    seek(&mut h);
    click_item(&mut h, video_a);
    h.key_press(egui::Key::Q);
    h.run_steps(2);
    assert_eq!(
        count(h.state()),
        4,
        "Q replaces the left piece with the right"
    );
    assert!(
        !h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .any(|item| item.id == video_a),
        "split-left removes the left piece"
    );
    let kept = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.start == Tick(2 * SECOND))
        .unwrap()
        .id;
    assert_eq!(
        item(h.state(), kept).source_range,
        SourceRange::new(Tick(2 * SECOND), Tick(6 * SECOND)).unwrap()
    );
    undo(&mut h);
    assert!(item(h.state(), video_a).source_range.start() == Tick::ZERO);

    seek(&mut h);
    click_item(&mut h, video_a);
    h.key_press(egui::Key::W);
    h.run_steps(2);
    assert_eq!(count(h.state()), 4, "W removes the right piece");
    assert_eq!(
        item(h.state(), video_a).source_range,
        SourceRange::new(Tick::ZERO, Tick(2 * SECOND)).unwrap()
    );
    undo(&mut h);

    click_item(&mut h, video_a);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(2);
    let overlays = &h.state().session.project().data().scene().timeline.overlays;
    assert_eq!(overlays[0].name.as_str(), "Video (copy)");
    let copy = h.state().inspect().unwrap().selection;
    assert_eq!(copy.len(), 1);
    assert_ne!(copy[0], video_a);
    assert_eq!(item(h.state(), copy[0]).start, Tick::ZERO);
    undo(&mut h);
    assert_eq!(
        h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .len(),
        2
    );

    click_item(&mut h, video_b);
    h.key_press(egui::Key::Delete);
    h.run_steps(2);
    assert!(
        !h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .any(|item| item.id == video_b),
        "Delete removes the selected clip"
    );
    undo(&mut h);

    click_item(&mut h, video_a);
    open_item_menu(&mut h, video_a, "hide");
    assert!(item(h.state(), video_a).hidden, "menu hides the clip");
    undo(&mut h);
    assert!(!item(h.state(), video_a).hidden);

    // Split, duplicate and delete also run from the clip menu with one undo step.
    seek(&mut h);
    click_item(&mut h, video_a);
    open_item_menu(&mut h, video_a, "split");
    assert_eq!(count(h.state()), 5, "the clip menu splits at the playhead");
    undo(&mut h);
    assert_eq!(count(h.state()), 4);

    click_item(&mut h, video_a);
    open_item_menu(&mut h, video_a, "duplicate");
    assert_eq!(
        h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .len(),
        3,
        "the clip menu duplicates onto a new track"
    );
    let copy = h.state().inspect().unwrap().selection;
    assert_eq!(copy.len(), 1);
    undo(&mut h);
    assert_eq!(
        h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .overlays
            .len(),
        2
    );

    click_item(&mut h, video_b);
    open_item_menu(&mut h, video_b, "delete");
    assert!(item_opt(h.state(), video_b).is_none(), "the menu deletes");
    undo(&mut h);
    assert!(item_opt(h.state(), video_b).is_some());

    click_item(&mut h, audio);
    open_item_menu(&mut h, audio, "mute");
    assert!(
        matches!(&item(h.state(), audio).content, ItemParams::Audio(params) if params.muted),
        "menu mutes the audio clip"
    );
    undo(&mut h);
    assert!(matches!(&item(h.state(), audio).content, ItemParams::Audio(params) if !params.muted));

    click_item(&mut h, video_a);
    open_item_menu(&mut h, video_a, "audio");
    assert!(
        matches!(
            &item(h.state(), video_a).content,
            ItemParams::Video(params) if !params.source_audio_enabled
        ),
        "menu extracts source audio"
    );
    let range = item(h.state(), video_a).source_range;
    let extracted = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .audio
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id != audio && item.source_range == range)
        .expect("extracted audio item keeps the source range")
        .id;
    undo(&mut h);
    assert!(matches!(
        &item(h.state(), video_a).content,
        ItemParams::Video(params) if params.source_audio_enabled
    ));
    redo(&mut h);
    open_item_menu(&mut h, video_a, "audio");
    assert!(
        matches!(
            &item(h.state(), video_a).content,
            ItemParams::Video(params) if params.source_audio_enabled
        ),
        "menu recovers source audio"
    );
    assert!(matches!(
        item(h.state(), extracted).content,
        ItemParams::Audio(_)
    ));

    // Reopen the clip menu for the audit screenshot.
    h.get(By::new().predicate(move |node| {
        node.author_id() == Some(format!("timeline.item.{}", video_a.0).as_str())
    }))
    .click_secondary();
    h.run_steps(2);
    let image = h.render().unwrap();
    let target =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-16-actions.png");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    image.save(&target).unwrap();
    eprintln!("clip actions screenshot: {}", target.display());
}

fn fades(editor: &Editor, id: ItemId) -> project::Fades {
    match &item(editor, id).content {
        ItemParams::Audio(params) => params.fades,
        _ => panic!("item {id:?} is not audio"),
    }
}

fn fade_handle(id: ItemId, edge: &str) -> String {
    format!("timeline.item.{}.{edge}", id.0)
}

/// Press a fade handle and move the pointer by `travel` seconds without releasing.
fn grab_fade(harness: &mut Harness<'_, Editor>, id: ItemId, edge: &str, travel: f32) -> egui::Pos2 {
    let zoom = harness.state().inspect().unwrap().zoom;
    let handle = widget(harness, &fade_handle(id, edge));
    let from = handle.center();
    let to = egui::pos2(from.x + travel * zoom, from.y);
    harness.drag_at(from);
    harness.run_steps(1);
    harness.hover_at(to);
    harness.run_steps(2);
    to
}

#[test]
fn fade_handles_drag_one_step_clamp_and_choose_a_curve() {
    let fixture = fixture();
    let video = fixture.video_a;
    let audio = audio_item(&fixture.editor);
    let text = text_item_id(&fixture.editor);
    let mut h = harness(fixture.editor);
    h.run_steps(3);
    let widgets = h.state().inspect().unwrap().widgets;
    for edge in ["fade_in", "fade_out"] {
        assert!(
            widgets
                .iter()
                .any(|widget| widget.id == fade_handle(audio, edge)),
            "audio clips expose a {edge} handle"
        );
        for other in [video, text] {
            assert!(
                !widgets
                    .iter()
                    .any(|widget| widget.id == fade_handle(other, edge)),
                "non-audio item {} must not paint a {edge} handle",
                other.0
            );
        }
    }
    let undo = |h: &mut Harness<'_, Editor>| {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
    };
    let redo = |h: &mut Harness<'_, Editor>| {
        h.key_press_modifiers(
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
            egui::Key::Z,
        );
        h.run_steps(2);
    };

    // Escape cancels a drag: the committed bytes, the history and the stored fade
    // all stay at the entry state, and no gesture is left open.
    let committed = h.state().session.project().clone();
    let bytes = serde_json::to_vec(committed.data()).unwrap();
    let history = h.state().session.history_len();
    let to = grab_fade(&mut h, audio, "fade_in", 2.0);
    assert_eq!(fades(h.state(), audio).fade_in.duration, Tick(2 * SECOND));
    let guiding = h.state().inspect().unwrap();
    assert!(
        guiding.hover_time.is_some(),
        "fade drags retain the preview-axis guide"
    );
    assert_eq!(guiding.preview_at, guiding.hover_time);
    assert_eq!(guiding.position, Tick::ZERO);
    assert!(h.state().session.gesture_active());
    assert_eq!(
        h.state().session.history_len(),
        history,
        "the drag stages one engine gesture and publishes nothing yet"
    );
    assert!(
        !h.state().session.can_undo(),
        "an open gesture is not an undo step"
    );
    assert!(h.state().session.transaction_active());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(
        !h.state().session.gesture_active(),
        "Escape closes the gesture"
    );
    assert_eq!(fades(h.state(), audio).fade_in.duration, Tick::ZERO);
    assert_eq!(
        h.state().session.project(),
        &committed,
        "Escape restores the entry project byte for byte"
    );
    assert_eq!(
        serde_json::to_vec(h.state().session.project().data()).unwrap(),
        bytes
    );
    assert_eq!(h.state().session.history_len(), history);
    h.drop_at(to);
    h.run_steps(2);

    // A drag publishes exactly one undo step through `audio.set_fade`.
    let to = grab_fade(&mut h, audio, "fade_in", 2.0);
    h.drop_at(to);
    h.run_steps(2);
    assert_eq!(fades(h.state(), audio).fade_in.duration, Tick(2 * SECOND));
    assert_eq!(h.state().session.history_len().0, history.0 + 1);
    undo(&mut h);
    assert_eq!(fades(h.state(), audio).fade_in.duration, Tick::ZERO);
    redo(&mut h);
    assert_eq!(fades(h.state(), audio).fade_in.duration, Tick(2 * SECOND));
    let history = h.state().session.history_len();

    // A 3 s fade-out leaves the fade-in only 7 s of the 10 s clip.
    let to = grab_fade(&mut h, audio, "fade_out", -3.0);
    h.drop_at(to);
    h.run_steps(2);
    assert_eq!(fades(h.state(), audio).fade_out.duration, Tick(3 * SECOND));
    assert_eq!(h.state().session.history_len().0, history.0 + 1);
    let to = grab_fade(&mut h, audio, "fade_in", 20.0);
    h.drop_at(to);
    h.run_steps(2);
    assert_eq!(
        fades(h.state(), audio).fade_in.duration,
        Tick(7 * SECOND),
        "the engine clamps the fade against the other edge"
    );

    // Without a fade-out the same drag fills the clip and stops at its length.
    let to = grab_fade(&mut h, audio, "fade_out", 3.0);
    h.drop_at(to);
    h.run_steps(2);
    assert_eq!(fades(h.state(), audio).fade_out.duration, Tick::ZERO);
    let to = grab_fade(&mut h, audio, "fade_in", 20.0);
    h.drop_at(to);
    h.run_steps(2);
    assert_eq!(
        fades(h.state(), audio).fade_in.duration,
        Tick(10 * SECOND),
        "the engine clamps the fade at the clip duration"
    );
    // A drag past the clip's start cannot store a negative fade.
    let to = grab_fade(&mut h, audio, "fade_in", -20.0);
    h.drop_at(to);
    h.run_steps(2);
    assert_eq!(fades(h.state(), audio).fade_in.duration, Tick::ZERO);
    undo(&mut h);
    assert_eq!(fades(h.state(), audio).fade_in.duration, Tick(10 * SECOND));

    // The handle menu re-shapes the edge with one `audio.set_fade`.
    let before = h.state().session.history_len();
    let handle = fade_handle(audio, "fade_in");
    h.get(By::new().predicate(move |node| node.author_id() == Some(handle.as_str())))
        .click_secondary();
    h.run_steps(2);
    let entry = fade_handle(audio, "fade_in.curve.exponential");
    h.get(By::new().predicate(move |node| node.author_id() == Some(entry.as_str())))
        .click();
    h.run_steps(3);
    let chosen = fades(h.state(), audio);
    assert_eq!(chosen.fade_in.curve, project::FadeCurve::Exponential);
    assert_eq!(chosen.fade_in.duration, Tick(10 * SECOND));
    assert_eq!(h.state().session.history_len().0, before.0 + 1);
    undo(&mut h);
    assert_eq!(
        fades(h.state(), audio).fade_in.curve,
        project::FadeCurve::EqualPower
    );

    for edge in ["fade_in", "fade_out"] {
        for (name, curve) in [
            ("linear", project::FadeCurve::Linear),
            ("exponential", project::FadeCurve::Exponential),
            ("equal_power", project::FadeCurve::EqualPower),
        ] {
            let before = h.state().session.project().clone();
            let history = h.state().session.history_len();
            let handle = fade_handle(audio, edge);
            h.get(By::new().predicate(move |node| node.author_id() == Some(handle.as_str())))
                .click_secondary();
            h.run_steps(2);
            let entry = fade_handle(audio, &format!("{edge}.curve.{name}"));
            h.get(By::new().predicate(move |node| node.author_id() == Some(entry.as_str())))
                .click();
            h.run_steps(2);
            let fade = if edge == "fade_in" {
                fades(h.state(), audio).fade_in
            } else {
                fades(h.state(), audio).fade_out
            };
            assert_eq!(fade.curve, curve);
            assert_eq!(h.state().session.history_len().0, history.0 + 1);
            undo(&mut h);
            assert_eq!(h.state().session.project(), &before);
            redo(&mut h);
        }
    }

    // A visible curve for each edge in the audit screenshot.
    let to = grab_fade(&mut h, audio, "fade_in", -7.0);
    h.drop_at(to);
    h.run_steps(2);
    let to = grab_fade(&mut h, audio, "fade_out", -3.0);
    h.drop_at(to);
    h.run_steps(2);
    assert_eq!(fades(h.state(), audio).fade_in.duration, Tick(3 * SECOND));
    assert_eq!(fades(h.state(), audio).fade_out.duration, Tick(3 * SECOND));
    let image = h.render().unwrap();
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    let path = target.join("p1-66-fades.png");
    image.save(&path).unwrap();
    eprintln!("fade screenshot: {}", path.display());
}

#[test]
fn fade_handles_short_clips_clamp_without_moving_volume_or_keyframes() {
    for duration in [Tick(SECOND / 2), Tick(SECOND / 30)] {
        let mut f = fixture();
        let audio = audio_item(&f.editor);
        assert!(
            f.editor
                .session
                .apply(Command::TrimEnd {
                    item: audio,
                    at: Tick::ZERO
                })
                .is_err(),
            "zero-length audio clips are refused by the consumed contract"
        );
        f.editor
            .session
            .apply(Command::TrimEnd {
                item: audio,
                at: duration,
            })
            .unwrap();
        f.editor
            .session
            .apply(Command::SetViewState {
                zoom: 3.0 / duration.seconds(),
                scroll: 0.0,
                playhead: Tick::ZERO,
            })
            .unwrap();
        f.editor
            .session
            .apply(Command::SetAnimatedParam {
                item: audio,
                path: project::BuiltinPath::Volume.into(),
                param: project::Param::new(project::ParamValue::Scalar(-60.0)).unwrap(),
            })
            .unwrap();
        f.editor
            .session
            .apply(Command::AddKeyframe {
                item: audio,
                path: project::BuiltinPath::Volume.into(),
                component: None,
                key: project::Keyframe::new(Tick::ZERO, project::ParamValue::Scalar(-60.0)),
            })
            .unwrap();
        let animations = item(&f.editor, audio).animations.clone();
        let source = item(&f.editor, audio).source_range;
        let mut h = harness(f.editor);
        h.run_steps(3);
        let lane = format!("timeline.item.{}.lanes", audio.0);
        h.get(By::new().predicate(move |node| node.author_id() == Some(lane.as_str())))
            .click();
        h.run_steps(2);
        let history = h.state().session.history_len();
        // Grab where the fade target overlaps the zero-time volume diamond.
        let from = widget(&h, &fade_handle(audio, "fade_in")).center() + egui::vec2(-6.0, 6.0);
        let to = from + egui::vec2(h.state().inspect().unwrap().zoom, 0.0);
        h.drag_at(from);
        h.run_steps(1);
        h.hover_at(to);
        h.run_steps(2);
        h.drop_at(to);
        h.run_steps(2);
        assert_eq!(fades(h.state(), audio).fade_in.duration, duration);
        assert_eq!(h.state().session.history_len().0, history.0 + 1);
        let to = grab_fade(&mut h, audio, "fade_out", -1.0);
        h.drop_at(to);
        h.run_steps(2);
        assert_eq!(fades(h.state(), audio).fade_out.duration, Tick::ZERO);
        assert_eq!(
            h.state().session.history_len().0,
            history.0 + 1,
            "no room means no history"
        );
        assert_eq!(item(h.state(), audio).source_range, source);
        assert_eq!(item(h.state(), audio).animations, animations);
        assert_eq!(item(h.state(), audio).start, Tick::ZERO);
        assert!(!h.state().session.gesture_active());
    }
}

#[test]
fn fade_handles_split_offsets_reverse_without_history_and_abort_exactly() {
    for edge in [project::FadeEdge::In, project::FadeEdge::Out] {
        let mut f = fixture();
        let original = audio_item(&f.editor);
        f.editor
            .session
            .apply(Command::SetFade {
                item: original,
                edge,
                duration: Tick(8 * SECOND),
                curve: project::FadeCurve::Linear,
            })
            .unwrap();
        let cut = f
            .editor
            .session
            .apply(Command::SplitAt {
                item: original,
                at: Tick(if edge == project::FadeEdge::In { 2 } else { 8 } * SECOND),
            })
            .unwrap();
        let audio = if edge == project::FadeEdge::In {
            cut.created
                .iter()
                .find_map(|id| match id {
                    CreatedId::Item(id) => Some(*id),
                    _ => None,
                })
                .unwrap()
        } else {
            original
        };
        let edge_id = if edge == project::FadeEdge::In {
            "fade_in"
        } else {
            "fade_out"
        };
        let mut h = harness(f.editor);
        h.run_steps(3);
        let entry = h.state().session.project().clone();
        let history = h.state().session.history_len();
        let from = widget(&h, &fade_handle(audio, edge_id)).center();
        let travel = if edge == project::FadeEdge::In {
            -1.0
        } else {
            1.0
        };
        grab_fade(&mut h, audio, edge_id, travel);
        assert_eq!(h.state().session.history_len(), history);
        assert_eq!(
            h.state().session.project(),
            &entry,
            "committed content stays untouched"
        );
        assert_ne!(h.state().session.preview_project().unwrap(), &entry);
        h.hover_at(from);
        h.run_steps(2);
        h.drop_at(from);
        h.run_steps(2);
        assert_eq!(
            h.state().session.project(),
            &entry,
            "{edge:?} reversal retains the split offset"
        );
        assert_eq!(h.state().session.history_len(), history);

        for abort in [
            "escape",
            "focus",
            "removed",
            "locked",
            "external_cancel",
            "fullscreen",
        ] {
            let to = grab_fade(&mut h, audio, edge_id, travel);
            assert!(h.state().session.gesture_active());
            match abort {
                "escape" => h.key_press(egui::Key::Escape),
                "focus" => {
                    // Match egui-winit: native focus updates both raw state and event.
                    h.input_mut().focused = false;
                    h.event(egui::Event::WindowFocused(false));
                }
                "removed" => {
                    h.state_mut()
                        .session
                        .apply(Command::Delete { items: vec![audio] })
                        .unwrap();
                }
                "locked" => {
                    let track = item_track(h.state(), audio);
                    h.state_mut()
                        .session
                        .apply(Command::SetTrackLocked {
                            track,
                            locked: true,
                        })
                        .unwrap();
                }
                "external_cancel" => {
                    h.state_mut().session.cancel_gesture().unwrap();
                }
                "fullscreen" => {
                    h.state_mut().activate("preview.fullscreen").unwrap();
                }
                _ => panic!("unknown case"),
            }
            h.run_steps(2);
            assert!(
                !h.state().session.gesture_active(),
                "{edge:?} {abort} closes the gesture"
            );
            assert_eq!(
                h.state().session.project(),
                &entry,
                "{edge:?} {abort} restores entry"
            );
            assert_eq!(h.state().session.history_len(), history);
            if abort == "fullscreen" {
                h.key_press(egui::Key::Escape);
                h.run_steps(2);
            }
            h.input_mut().focused = true;
            h.event(egui::Event::WindowFocused(true));
            h.drop_at(to);
            h.run_steps(2);
            assert_eq!(
                h.state().session.project(),
                &entry,
                "a stale release cannot edit"
            );
        }
        // The other edge sees only the audible 6 s window of the 8 s split fade.
        let other = if edge == project::FadeEdge::In {
            "fade_out"
        } else {
            "fade_in"
        };
        let to = grab_fade(&mut h, audio, other, 10.0 * travel);
        h.drop_at(to);
        h.run_steps(2);
        let remaining = fades(h.state(), audio);
        let changed = if edge == project::FadeEdge::In {
            remaining.fade_out
        } else {
            remaining.fade_in
        };
        assert_eq!(
            changed.duration,
            Tick(2 * SECOND),
            "clamp against the split window, not its whole duration"
        );
        assert_eq!(h.state().session.history_len().0, history.0 + 1);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run_steps(2);
        assert_eq!(h.state().session.project(), &entry);

        // A surviving handle remains usable after every abort.
        let to = grab_fade(&mut h, audio, edge_id, travel);
        h.drop_at(to);
        h.run_steps(2);
        assert_eq!(h.state().session.history_len().0, history.0 + 1);
    }
}

#[test]
fn clipboard_keys_menus_zero_key_target_refusal_undo_and_bilingual_captures() {
    for locale in [Locale::En, Locale::Vi] {
        for scale in [1.0, 2.0] {
            let mut session = Session::new(
                Project::empty(ProjectId(399), "Clipboard".into()).unwrap(),
                50,
            )
            .unwrap();
            let video = track(&mut session, "Video", TrackKind::Video);
            let source = insert(
                &mut session,
                video,
                Tick::ZERO,
                SourceRange::new(Tick::ZERO, Tick(6 * SECOND)).unwrap(),
                None,
                ItemParams::Overlay(project::OverlayParams {
                    color: Color([255; 4]),
                }),
            );
            let target = insert(
                &mut session,
                video,
                Tick(6 * SECOND),
                SourceRange::new(Tick::ZERO, Tick(4 * SECOND)).unwrap(),
                None,
                ItemParams::Overlay(project::OverlayParams {
                    color: Color([255, 80, 30, 255]),
                }),
            );
            session
                .apply(Command::SetAnimatedParam {
                    item: target,
                    path: project::BuiltinPath::Opacity.into(),
                    param: project::Param::new(project::ParamValue::Scalar(0.2)).unwrap(),
                })
                .unwrap();
            let mut editor = editor_for(
                (**session.project()).clone(),
                Preferences {
                    locale,
                    timeline_lanes: std::collections::BTreeMap::from([(
                        ProjectId(399),
                        std::collections::BTreeSet::from([target.0]),
                    )]),
                    ..Default::default()
                },
            );
            add_key(&mut editor, source, "opacity", Tick(2 * SECOND), 0.25);
            add_key(&mut editor, source, "opacity", Tick(4 * SECOND), 0.75);
            let color_path = project::AnimationPath::new("params.strokeColor").unwrap();
            let mut color =
                project::Param::new(project::ParamValue::Color([0.8, 0.2, 0.1, 0.25])).unwrap();
            color
                .channel_mut(Some(project::ColorComponent::R))
                .unwrap()
                .put(project::Keyframe::new(
                    Tick(2 * SECOND),
                    project::ParamValue::Scalar(0.8),
                ))
                .unwrap();
            color
                .channel_mut(Some(project::ColorComponent::A))
                .unwrap()
                .put(project::Keyframe::new(
                    Tick(4 * SECOND),
                    project::ParamValue::Scalar(0.25),
                ))
                .unwrap();
            editor
                .session
                .apply(Command::SetAnimatedParam {
                    item: source,
                    path: color_path.clone(),
                    param: color,
                })
                .unwrap();
            editor
                .session
                .apply(Command::SetAnimatedParam {
                    item: target,
                    path: color_path.clone(),
                    param: project::Param::new(project::ParamValue::Color([0.0; 4])).unwrap(),
                })
                .unwrap();
            let mut h = crate::gpu::builder()
                .with_size(egui::vec2(1727.0, 1029.0))
                .with_pixels_per_point(scale)
                .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor);
            h.run_steps(3);
            let lanes = widget(&h, &format!("timeline.item.{}.lanes", source.0));
            click_at(&mut h, lanes.center());
            h.run_steps(2);
            let first = widget(
                &h,
                &format!("timeline.keyframe.{}.opacity.none.{}", source.0, 2 * SECOND),
            );
            click_at(&mut h, first.center());
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
            let last = widget(
                &h,
                &format!("timeline.keyframe.{}.opacity.none.{}", source.0, 4 * SECOND),
            );
            click_at(&mut h, last.center());
            for (component, time) in [("r", 2 * SECOND), ("a", 4 * SECOND)] {
                let key = widget(
                    &h,
                    &format!(
                        "timeline.keyframe.{}.params_strokeColor.{component}.{time}",
                        source.0
                    ),
                );
                click_at(&mut h, key.center());
            }
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            assert!(h.state().action_enabled("edit.copy"));
            let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
            std::fs::create_dir_all(&directory).unwrap();
            let language = if locale == Locale::En { "en" } else { "vi" };
            let capture = |h: &mut Harness<'_, Editor>, state: &str| {
                h.event(egui::Event::PointerMoved(egui::pos2(800.0, 25.0)));
                h.run_steps(2);
                h.state_mut()
                    .settle_previews(vibecut::SETTLE_BUDGET)
                    .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while !h.state().playback.frame().is_some_and(|f| {
                    f.generation == h.state().playback.generation()
                        && f.at == h.state().playback.position().unwrap()
                }) {
                    h.run_steps(1);
                    assert!(
                        std::time::Instant::now() < deadline,
                        "presented clipboard frame timeout"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                h.run_steps(2);
                let path =
                    directory.join(format!("p1-20-clipboard-{language}-{scale}x-{state}.png"));
                h.render().unwrap().save(&path).unwrap();
                if state == "items" && language == "en" && scale == 1.0 {
                    std::fs::copy(&path, directory.join("p1-20-clipboard.png")).unwrap();
                }
            };
            capture(&mut h, "before");
            let before = h.state().session.project().clone();
            let history = h.state().session.history_len();
            h.input_mut().events.push(egui::Event::Key {
                key: egui::Key::C,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            });
            h.step();
            assert!(
                !h.output().platform_output.commands.iter().any(|c| matches!(
                    c,
                    egui::OutputCommand::CopyText(_) | egui::OutputCommand::CopyImage(_)
                ))
            );
            h.run_steps(2);
            assert!(h.state().session.clipboard_has_entry());
            assert_eq!(h.state().session.project(), &before);
            assert_eq!(h.state().session.history_len(), history);
            h.get(By::new().predicate(|n| {
                n.author_id() == Some(format!("timeline.lane.{}.opacity", source.0).as_str())
            }))
            .click_secondary();
            h.run_steps(2);
            capture(&mut h, "copy");
            h.get(By::new().predicate(|n| {
                n.author_id()
                    == Some(format!("timeline.lane.{}.opacity.none.copy", source.0).as_str())
            }))
            .click();
            h.run_steps(2);
            assert_eq!(h.state().session.project(), &before);
            click_item(&mut h, target);
            h.state_mut()
                .dispatch(AppCommand::Seek(Tick(7 * SECOND)))
                .unwrap();
            h.run_steps(2);
            assert!(h.state().action_enabled("edit.paste"));
            let before = h.state().session.project().clone();
            let history = h.state().session.history_len().0;
            open_item_menu(&mut h, target, "paste");
            let channel = item(h.state(), target)
                .animations
                .get(&project::BuiltinPath::Opacity.into())
                .unwrap()
                .channel(None)
                .unwrap();
            assert_eq!(
                channel.keys().iter().map(|k| k.time.0).collect::<Vec<_>>(),
                [SECOND, 3 * SECOND]
            );
            assert_eq!(channel.keys()[0].value, project::ParamValue::Scalar(0.25));
            assert_eq!(channel.keys()[1].value, project::ParamValue::Scalar(0.75));
            let color = item(h.state(), target).animations.get(&color_path).unwrap();
            assert_eq!(
                color
                    .channel(Some(project::ColorComponent::R))
                    .unwrap()
                    .keys()[0]
                    .time,
                Tick(SECOND)
            );
            assert_eq!(
                color
                    .channel(Some(project::ColorComponent::A))
                    .unwrap()
                    .keys()[0]
                    .time,
                Tick(3 * SECOND)
            );
            assert_eq!(
                color
                    .channel(Some(project::ColorComponent::G))
                    .unwrap()
                    .base(),
                &project::ParamValue::Scalar(0.0)
            );

            assert_eq!(h.state().session.history_len().0, history + 1);
            assert_eq!(h.state().playback.position().unwrap(), Tick(7 * SECOND));
            capture(&mut h, "paste");
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.run_steps(2);
            assert_eq!(h.state().session.project(), &before);
            assert!(
                h.state()
                    .inspect()
                    .unwrap()
                    .expanded_lanes
                    .contains(&source)
            );
            capture(&mut h, "undo");
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::V);
            h.run_steps(2);
            assert_eq!(
                item(h.state(), target)
                    .animations
                    .get(&project::BuiltinPath::Opacity.into())
                    .unwrap()
                    .channel(None)
                    .unwrap()
                    .keys()
                    .len(),
                2
            );
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.run_steps(2);
            assert_eq!(h.state().session.project(), &before);
            // Item scope: Ctrl+C copies the selected item, Ctrl+V pastes at the
            // playhead on its relative track, selects the new item and advances.
            // Seek clamps to the project end, so paste there: free of any clip.
            let end = h.state().session.project().duration();
            h.state_mut().dispatch(AppCommand::Seek(end)).unwrap();
            h.run_steps(2);
            click_item(&mut h, source);
            let items_before = h.state().session.project().clone();
            let item_history = h.state().session.history_len().0;
            let source_item = item(h.state(), source).clone();
            assert!(h.state().action_enabled("edit.copy"));
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::C);
            h.step();
            assert!(
                !h.output().platform_output.commands.iter().any(|c| matches!(
                    c,
                    egui::OutputCommand::CopyText(_) | egui::OutputCommand::CopyImage(_)
                )),
                "item payloads never reach the OS clipboard"
            );
            h.run_steps(2);
            assert!(h.state().session.clipboard_has_entry());
            assert_eq!(h.state().session.project(), &items_before);
            assert_eq!(h.state().session.history_len().0, item_history);
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::V);
            h.run_steps(2);
            assert_eq!(h.state().session.history_len().0, item_history + 1);
            assert_eq!(h.state().selection.len(), 1);
            let pasted = *h.state().selection.iter().next().unwrap();
            assert_eq!(item(h.state(), pasted).start, end);
            assert_eq!(item(h.state(), pasted).content, source_item.content);
            assert_eq!(
                h.state().playback.position().unwrap(),
                (end + source_item.duration().unwrap()).min(h.state().session.project().duration())
            );
            capture(&mut h, "items");
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.run_steps(2);
            assert_eq!(h.state().session.project(), &items_before);
            // The item context menu dispatches the same Paste command.
            click_item(&mut h, target);
            h.state_mut().dispatch(AppCommand::Seek(end)).unwrap();
            h.run_steps(2);
            assert!(h.state().action_enabled("edit.paste"));
            open_item_menu(&mut h, target, "paste");
            assert_ne!(h.state().session.project(), &items_before);
            assert_eq!(h.state().session.history_len().0, item_history + 1);
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.run_steps(2);
            assert_eq!(h.state().session.project(), &items_before);
            h.state_mut()
                .session
                .apply(Command::SetTrackLocked {
                    track: video,
                    locked: true,
                })
                .unwrap();
            h.run_steps(2);
            let before = h.state().session.project().clone();
            let history = h.state().session.history_len();
            assert!(!h.state().action_enabled("edit.paste"));
            assert!(h.state_mut().activate("edit.paste").is_err());
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::V);
            h.run_steps(2);
            assert_eq!(h.state().session.project(), &before);
            assert_eq!(h.state().session.history_len(), history);
            capture(&mut h, "refused");
        }
    }
}

#[test]
fn clipboard_direct_actions_preserve_focused_property_drafts_and_text_paste_stays_text() {
    let f = fixture();
    let id = f.video_a;
    let mut editor = f.editor;
    add_key(&mut editor, id, "opacity", Tick(2 * SECOND), 0.5);
    editor
        .session
        .apply(Command::CopyKeyframes {
            keys: vec![engine::KeyframeRef {
                item: id,
                path: project::BuiltinPath::Opacity.into(),
                component: None,
                time: Tick(2 * SECOND),
            }],
        })
        .unwrap();
    let mut h = harness(editor);
    h.run_steps(3);
    click_item(&mut h, id);
    h.get(By::new().predicate(|n| n.author_id() == Some("inspector.group.basic")))
        .click();
    h.run_steps(2);
    h.get(By::new().predicate(|n| n.author_id() == Some("inspector.tab.blending")))
        .click();
    h.run_steps(2);
    h.get(By::new().predicate(|n| n.author_id() == Some("inspector.opacity")))
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("37".into()));
    h.run_steps(2);
    let before = h.state().session.project().clone();
    let rev = h.state().session.revision();
    let history = h.state().session.history_len();
    for id in ["edit.copy", "edit.paste"] {
        assert!(!h.state().action_enabled(id));
        assert!(h.state_mut().activate(id).is_err());
    }
    h.run_steps(2);
    assert_eq!(h.state().session.project(), &before);
    assert_eq!(h.state().session.revision(), rev);
    assert_eq!(h.state().session.history_len(), history);
    h.get(
        By::new()
            .value("37")
            .predicate(|n| n.author_id() == Some("inspector.opacity")),
    );
    // Returned text remains a field edit even while an in-app key snapshot exists.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Paste("42".into()));
    h.run_steps(2);
    assert_eq!(h.state().session.project(), &before);
    assert_eq!(h.state().session.history_len(), history);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(
        item(h.state(), id)
            .animations
            .get(&project::BuiltinPath::Opacity.into())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .iter()
            .find(|k| k.time == Tick::ZERO)
            .map(|k| &k.value),
        Some(&project::ParamValue::Scalar(0.42))
    );
    assert!(h.state().session.clipboard_has_entry());
}

#[test]
fn clipboard_oversized_held_text_selection_disables_copy_and_keyboard_refuses_without_state_changes()
 {
    let f = fixture();
    let id = f.video_a;
    let mut editor = f.editor;
    add_key(&mut editor, id, "opacity", Tick(2 * SECOND), 0.5);
    editor
        .session
        .apply(Command::CopyKeyframes {
            keys: vec![engine::KeyframeRef {
                item: id,
                path: project::BuiltinPath::Opacity.into(),
                component: None,
                time: Tick(2 * SECOND),
            }],
        })
        .unwrap();
    let path = project::AnimationPath::new("params.text").unwrap();
    let mut key = project::Keyframe::new(
        Tick::ZERO,
        project::ParamValue::Text("x".repeat(600_000).into()),
    );
    key.segment = project::Segment::Step;
    editor
        .session
        .apply(Command::SetAnimatedParam {
            item: id,
            path: path.clone(),
            param: project::Param::Value(
                project::Channel::new(
                    project::ParamValue::Text("b".repeat(600_000).into()),
                    vec![key],
                    project::Extrapolation::Hold,
                    project::Extrapolation::Hold,
                )
                .unwrap(),
            ),
        })
        .unwrap();
    let mut h = harness(editor);
    h.run_steps(3);
    click_item(&mut h, id);
    h.state_mut()
        .selection_scopes
        .keyframes
        .insert(vibecut::selection::KeyframeSelection {
            item: id,
            path,
            component: None,
            time: Tick::ZERO,
        });
    h.run_steps(3);
    let before = h.state().session.project().clone();
    let rev = h.state().session.revision();
    let history = h.state().session.history_len();
    assert_eq!(
        h.state().inspect().unwrap().selection_scope,
        vibecut::selection::SelectionScope::Keyframes
    );
    assert!(!h.state().action_enabled("edit.copy"));
    assert!(h.state_mut().activate("edit.copy").is_err());
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::C,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.step();
    assert!(
        !h.output()
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c, egui::OutputCommand::CopyText(_)))
    );
    h.run_steps(3);
    assert!(h.state().session.clipboard_has_entry());
    assert_eq!(h.state().session.project(), &before);
    assert_eq!(h.state().session.revision(), rev);
    assert_eq!(h.state().session.history_len(), history);
    // The same oversized Text is unrelated to the small retained opacity snapshot.
    // Real frame admission and action dispatch must allow that Paste with one undo.
    assert!(h.state().action_enabled("edit.paste"));
    h.state_mut().activate("edit.paste").unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.history_len().0, history.0 + 1);
    let destination = item(h.state(), id);
    let old_item = before
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == id)
        .unwrap();
    assert_eq!(
        destination
            .animations
            .get(&project::AnimationPath::new("params.text").unwrap()),
        old_item
            .animations
            .get(&project::AnimationPath::new("params.text").unwrap())
    );
    assert_eq!(
        destination
            .animations
            .get(&project::BuiltinPath::Opacity.into())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()[0]
            .value,
        project::ParamValue::Scalar(0.5)
    );
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.project(), &before);
    assert!(h.state().session.clipboard_has_entry());
}
