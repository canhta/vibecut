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
use media::{
    AssetData, AssetId, AudioInfo, ColorInfo, ContentHash, SourceAsset, SourceProbe, SourceStream,
    TimeBase, VideoInfo,
};
use project::{Color, ItemId, ItemParams, Project, ProjectId, TrackId, TrackKind};
use std::{path::PathBuf, sync::Arc};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor,
    catalog::{Locale, Preferences},
    egui,
    playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;

pub(crate) struct Fixture {
    pub(crate) editor: Editor,
    pub(crate) video_a: ItemId,
    pub(crate) video_b: ItemId,
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
        ephemeral: false,
        id: AssetId(id),
        path: std::env::temp_dir().join(format!("vibecut-timeline-{id}.mp4")),
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

pub(crate) fn harness(editor: Editor) -> Harness<'static, Editor> {
    Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
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
    harness.event(egui::Event::Text("0.5".into()));
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
        .project()
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
    let store = vibecut::catalog::PreferenceStore::new(dir.path().join("preferences.json"));
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
    assert_ne!(harness.state().session.project(), &committed);
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
        ("timeline.snap", "magnet"),
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
        (split.rect.height() - 24.0).abs() < 0.5 && (split.rect.width() - 24.0).abs() < 0.5,
        "timeline actions use the measured 24x24 icon-only hit box, got {:?}",
        split.rect
    );
    let timeline = harness.state().panels["timeline"];
    assert!(
        (split.rect.min.y - timeline.top() - 16.0).abs() < 0.5,
        "timeline toolbar uses its measured 32 px row within the existing panel inset"
    );
    let undo = format!(
        "Undo ({})",
        vibecut::actions::Chord::parse("Ctrl+Z")
            .unwrap()
            .display(vibecut::actions::Platform::current())
    );
    for (id, text) in [
        ("edit.undo", undo.as_str()),
        ("timeline.split", "Split (S)"),
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
        .scene()
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
fn review_trim_release_survives_its_handle_becoming_narrow() {
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
    // An adapter or scene restoration replaces the navigation context, including
    // the transient pre-anchor position from an earlier zoom gesture.
    for (zoom, scroll) in [(1.0, 80.0), (25.0, 2000.0)] {
        fixture
            .editor
            .session
            .execute(
                "project.set_view_state",
                serde_json::json!({"zoom":zoom,"scroll":scroll,"playhead":0}),
            )
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
    let mut harness = Harness::builder()
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
    let mut harness = Harness::builder()
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
    let fields = vibecut::selection::shared_fields(
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
            let mut harness = Harness::builder()
                .with_size(egui::vec2(width, 900.0))
                .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
            harness.run_steps(3);
            let button = widget(&harness, "text.insert");
            assert!(
                harness.state().panels["media"].contains_rect(button),
                "{locale:?} at {width}: {button:?} outside {:?}",
                harness.state().panels["media"]
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
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 720.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
    harness.run_steps(3);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("text.insert")))
        .click();
    harness.run_steps(3);
    let panel = harness.state().panels["inspector"];
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(panel.center()));
    harness.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -2000.0),
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
    let mut harness = Harness::builder()
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
        let mut h = Harness::builder()
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
        drag(&mut h, body.center(), target);
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
        .execute(
            "project.set_view_state",
            serde_json::json!({"zoom":20,"scroll":4800,"playhead":0}),
        )
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
        .execute(
            "project.set_view_state",
            serde_json::json!({"zoom":1,"scroll":0.4,"playhead":0}),
        )
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
    let restored = h.state().playback.generation();
    assert!(
        restored > generation,
        "Escape must restart the preview generation"
    );
    // The restored frame is produced asynchronously. Wait for the state the
    // test needs instead of asserting an exact frame count: a frame from the
    // restored generation is valid whenever it lands, a frame from the
    // cancelled drag is not.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        h.run_steps(1);
        if let Some(frame) = h.state().playback.frame() {
            assert_eq!(
                frame.epoch, restored,
                "a cancelled-drag preview frame must not stay visible"
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the restored preview never produced a frame"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
