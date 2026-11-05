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
use engine::{ByteBudget, Command, CreatedId, NewItem, Placement, Session};
use media::{
    AssetData, AssetId, AudioInfo, ColorInfo, ContentHash, SourceAsset, SourceProbe, SourceStream,
    TimeBase, VideoInfo,
};
use project::{Color, ItemId, ItemParams, Project, ProjectId, TrackId, TrackKind};
use std::{path::PathBuf, sync::Arc};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor,
    catalog::{PreferenceStore, Preferences},
    egui,
    panels::{AssetsTab, ElementKind, PropertiesTab},
    playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;

struct Fixture {
    editor: Editor,
    text: ItemId,
    video: ItemId,
    silent_video: ItemId,
    image: ItemId,
    graphic: ItemId,
    audio: ItemId,
}

fn stream(kind: media::StreamKind, frame_count: Option<u64>) -> SourceStream {
    SourceStream {
        index: 0,
        kind,
        codec: match kind {
            media::StreamKind::Video => "h264".into(),
            media::StreamKind::Audio => "aac".into(),
            media::StreamKind::Other => "data".into(),
        },
        time_base: TimeBase::new(1, 1_000).unwrap(),
        duration: Some(Tick(10 * SECOND)),
        start: None,
        frame_rate: (kind == media::StreamKind::Video).then_some(FrameRate::FPS_30),
        frame_count,
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

fn asset(id: u64, mut streams: Vec<SourceStream>) -> SourceAsset {
    for (index, stream) in streams.iter_mut().enumerate() {
        stream.index = index;
    }
    SourceAsset::try_from(AssetData {
        id: AssetId(id),
        ephemeral: false,
        path: std::env::temp_dir().join(format!("vibecut-panels-{id}.mp4")),
        content_hash: ContentHash::from_bytes(&[id as u8; 8]),
        probe: SourceProbe {
            format: "mov,mp4".into(),
            duration: Some(Tick(10 * SECOND)),
            streams,
        },
    })
    .unwrap()
}

fn add_track(session: &mut Session, name: &str, kind: TrackKind) -> TrackId {
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

fn insert(
    session: &mut Session,
    track: TrackId,
    start: Tick,
    end: Tick,
    asset: Option<AssetId>,
    content: ItemParams,
) -> ItemId {
    let outcome = session
        .apply(Command::InsertItem {
            placement: Placement::Track { track },
            item: NewItem {
                animations: Default::default(),
                source_asset: asset,
                source_range: SourceRange::new(Tick::ZERO, end).unwrap(),
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

fn fixture() -> Fixture {
    let mut session =
        Session::new(Project::empty(ProjectId(6), "Panels".into()).unwrap(), 50).unwrap();
    for (id, streams) in [
        (
            1,
            vec![
                stream(media::StreamKind::Video, None),
                stream(media::StreamKind::Audio, None),
            ],
        ),
        (2, vec![stream(media::StreamKind::Audio, None)]),
        (3, vec![stream(media::StreamKind::Video, Some(1))]),
        (4, vec![stream(media::StreamKind::Video, None)]),
    ] {
        session
            .apply(Command::AddAsset {
                asset: asset(id, streams),
            })
            .unwrap();
    }
    let main = session.project().data().scene().timeline.main.id;
    let text_track = add_track(&mut session, "Text", TrackKind::Text);
    let audio_track = add_track(&mut session, "Audio", TrackKind::Audio);
    let video = insert(
        &mut session,
        main,
        Tick::ZERO,
        Tick(2 * SECOND),
        Some(AssetId(1)),
        ItemParams::Video(project::VideoParams {
            source_audio_enabled: true,
        }),
    );
    let graphic = insert(
        &mut session,
        main,
        Tick(2 * SECOND),
        Tick(SECOND),
        None,
        ItemParams::Overlay(project::OverlayParams {
            color: Color([10, 20, 30, 255]),
        }),
    );
    let image = insert(
        &mut session,
        main,
        Tick(3 * SECOND),
        Tick(SECOND),
        Some(AssetId(3)),
        ItemParams::Video(project::VideoParams {
            source_audio_enabled: false,
        }),
    );
    let silent_video = insert(
        &mut session,
        main,
        Tick(4 * SECOND),
        Tick(SECOND),
        Some(AssetId(4)),
        ItemParams::Video(project::VideoParams {
            source_audio_enabled: false,
        }),
    );
    let text = insert(
        &mut session,
        text_track,
        Tick::ZERO,
        Tick(2 * SECOND),
        None,
        ItemParams::Text(project::TextParams {
            content: "Hello".into(),
            font_size: 48.0,
            color: Color([255, 255, 255, 255]),
        }),
    );
    let audio = insert(
        &mut session,
        audio_track,
        Tick::ZERO,
        Tick(2 * SECOND),
        Some(AssetId(2)),
        ItemParams::Audio(project::AudioParams {
            gain_db: 0.0,
            muted: false,
        }),
    );
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Fixture {
        editor: Editor::new(session, playback, Preferences::default()),
        text,
        video,
        silent_video,
        image,
        graphic,
        audio,
    }
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn select(harness: &mut Harness<Editor>, item: ItemId) {
    harness.state_mut().selection = [item].into_iter().collect();
    harness.run_steps(3);
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    harness.run_steps(2);
}

fn has_widget(harness: &Harness<Editor>, id: &str) -> bool {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .any(|widget| widget.id == id)
}

fn widget_rect(harness: &Harness<Editor>, id: &str) -> egui::Rect {
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

fn item(harness: &Harness<Editor>, id: ItemId) -> project::Item {
    harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == id)
        .cloned()
        .unwrap()
}

fn visible_tabs(harness: &Harness<Editor>) -> Vec<String> {
    let mut tabs: Vec<_> = harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .filter(|widget| widget.id.starts_with("inspector.tab."))
        .collect();
    tabs.sort_by(|a, b| {
        (a.rect.top(), a.rect.left())
            .partial_cmp(&(b.rect.top(), b.rect.left()))
            .unwrap()
    });
    tabs.into_iter().map(|widget| widget.id).collect()
}

fn names(tabs: &[PropertiesTab]) -> Vec<String> {
    tabs.iter()
        .map(|tab| format!("inspector.tab.{}", tab.id()))
        .collect()
}

fn drag_to(harness: &mut Harness<Editor>, id: &str, delta: egui::Vec2) {
    let start = widget_rect(harness, id).center();
    harness.hover_at(start);
    harness.run_steps(1);
    harness.drag_at(start);
    harness.run_steps(1);
    harness.hover_at(start + delta);
    harness.run_steps(2);
}

fn drag(harness: &mut Harness<Editor>, id: &str, delta: egui::Vec2) {
    let start = widget_rect(harness, id).center();
    drag_to(harness, id, delta);
    harness.drop_at(start + delta);
    harness.run_steps(2);
}

#[test]
fn reachable_element_kinds_show_their_reference_tab_sets() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    for (item, expected, active_field) in [
        (
            fixture.text,
            ElementKind::Text.tabs(),
            Some("inspector.text_size"),
        ),
        (fixture.video, ElementKind::Video.tabs(), None),
        (fixture.image, ElementKind::Image.tabs(), None),
        (fixture.graphic, ElementKind::Graphic.tabs(), None),
        (
            fixture.audio,
            ElementKind::Audio.tabs(),
            Some("inspector.audio_gain"),
        ),
    ] {
        select(&mut harness, item);
        assert_eq!(visible_tabs(&harness), names(expected), "{item:?}");
        if let Some(field) = active_field {
            assert!(has_widget(&harness, field), "{item:?} default tab content");
        }
    }
    // A video with no audio stream hides its Audio tab.
    select(&mut harness, fixture.silent_video);
    let expected: Vec<_> = ElementKind::Video
        .tabs()
        .iter()
        .copied()
        .filter(|tab| *tab != PropertiesTab::Audio)
        .collect();
    assert_eq!(visible_tabs(&harness), names(&expected));
}

#[test]
fn every_registry_kind_has_its_reference_tab_set() {
    for (kind, expected) in [
        (
            ElementKind::Text,
            &[
                PropertiesTab::Text,
                PropertiesTab::Transform,
                PropertiesTab::Blending,
            ][..],
        ),
        (
            ElementKind::Video,
            &[
                PropertiesTab::Transform,
                PropertiesTab::Audio,
                PropertiesTab::Speed,
                PropertiesTab::Blending,
                PropertiesTab::Masks,
                PropertiesTab::Effects,
            ][..],
        ),
        (
            ElementKind::Image,
            &[
                PropertiesTab::Transform,
                PropertiesTab::Blending,
                PropertiesTab::Masks,
                PropertiesTab::Effects,
            ][..],
        ),
        (
            ElementKind::Sticker,
            &[
                PropertiesTab::Transform,
                PropertiesTab::Blending,
                PropertiesTab::Effects,
            ][..],
        ),
        (
            ElementKind::Graphic,
            &[
                PropertiesTab::Graphic,
                PropertiesTab::Transform,
                PropertiesTab::Blending,
                PropertiesTab::Masks,
                PropertiesTab::Effects,
            ][..],
        ),
        (
            ElementKind::Audio,
            &[PropertiesTab::Audio, PropertiesTab::Speed][..],
        ),
        (ElementKind::Effect, &[PropertiesTab::Effects][..]),
    ] {
        assert_eq!(kind.tabs(), expected, "{kind:?}");
        assert!(expected.contains(&kind.default_tab()), "{kind:?}");
    }
}

#[test]
fn active_tab_is_remembered_per_element_kind_across_reselection() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.text);
    click(&mut harness, "inspector.tab.transform");
    assert!(!has_widget(&harness, "inspector.text_size"));
    select(&mut harness, fixture.video);
    click(&mut harness, "inspector.tab.blending");
    assert!(has_widget(&harness, "inspector.opacity"));
    select(&mut harness, fixture.text);
    assert_eq!(
        visible_tabs(&harness),
        names(ElementKind::Text.tabs()),
        "text keeps its own tab, not the video's"
    );
    assert!(!has_widget(&harness, "inspector.text_size"));
    assert_eq!(
        harness
            .state()
            .preferences
            .properties_tabs
            .get("text")
            .map(String::as_str),
        Some("transform")
    );
    assert_eq!(
        harness
            .state()
            .preferences
            .properties_tabs
            .get("video")
            .map(String::as_str),
        Some("blending")
    );
}

#[test]
fn scrub_previews_then_commits_one_undo_step_and_escape_cancels() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.text);
    let history = harness.state().session.history_len();
    drag_to(
        &mut harness,
        "inspector.text_size.scrub",
        egui::vec2(40.0, 0.0),
    );
    assert!(harness.state().session.gesture_active());
    assert_eq!(harness.state().session.history_len(), history);
    assert_eq!(text_size(&harness, fixture.text), 68.0);
    let start = widget_rect(&harness, "inspector.text_size.scrub").center();
    harness.drop_at(start + egui::vec2(40.0, 0.0));
    harness.run_steps(2);
    assert!(!harness.state().session.gesture_active());
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert_eq!(text_size(&harness, fixture.text), 68.0);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(text_size(&harness, fixture.text), 48.0);
    harness.state_mut().activate("edit.redo").unwrap();
    harness.run_steps(2);
    assert_eq!(text_size(&harness, fixture.text), 68.0);

    // Escape during the drag reverts the preview without a history entry.
    let history = harness.state().session.history_len();
    drag_to(
        &mut harness,
        "inspector.text_size.scrub",
        egui::vec2(20.0, 0.0),
    );
    assert_eq!(text_size(&harness, fixture.text), 78.0);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.drop_at(
        widget_rect(&harness, "inspector.text_size.scrub").center() + egui::vec2(20.0, 0.0),
    );
    harness.run_steps(2);
    assert_eq!(text_size(&harness, fixture.text), 68.0);
    assert_eq!(harness.state().session.history_len(), history);
    assert!(!harness.state().session.gesture_active());
}

#[test]
fn scrub_reversal_to_origin_leaves_no_history() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.text);
    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "inspector.text_size.scrub").center();
    drag_to(
        &mut harness,
        "inspector.text_size.scrub",
        egui::vec2(40.0, 0.0),
    );
    assert_eq!(text_size(&harness, fixture.text), 68.0);
    harness.hover_at(start - egui::vec2(20.0, 0.0));
    harness.run_steps(2);
    assert_eq!(text_size(&harness, fixture.text), 38.0);
    harness.hover_at(start);
    harness.run_steps(2);
    harness.drop_at(start);
    harness.run_steps(2);
    assert_eq!(text_size(&harness, fixture.text), 48.0);
    assert_eq!(harness.state().session.history_len(), history);
    assert!(!harness.state().session.gesture_active());
}

#[test]
fn hiding_a_scrub_field_rolls_back_and_closes_gesture() {
    for change in ["selection", "command", "empty", "details", "tab"] {
        let fixture = fixture();
        let mut harness = build(fixture.editor);
        harness.run_steps(3);
        select(&mut harness, fixture.text);
        let history = harness.state().session.history_len();
        drag_to(
            &mut harness,
            "inspector.text_size.scrub",
            egui::vec2(40.0, 0.0),
        );
        assert_eq!(text_size(&harness, fixture.text), 68.0);
        match change {
            "selection" => select(&mut harness, fixture.audio),
            "command" => {
                harness
                    .state_mut()
                    .activate(&format!("timeline.item.{}", fixture.audio.0))
                    .unwrap();
                assert!(!harness.state().session.gesture_active());
                assert_eq!(
                    harness.state().session.selection(),
                    &harness.state().selection
                );
            }
            "empty" => harness.state_mut().selection.clear(),
            "details" => harness.state_mut().activate("inspector.details").unwrap(),
            "tab" => harness
                .state_mut()
                .preferences
                .set("properties_tabs", serde_json::json!({"text": "transform"}))
                .unwrap(),
            _ => panic!("unknown visibility boundary {change}"),
        }
        harness.run_steps(2);
        harness.drop_at(egui::pos2(700.0, 300.0));
        harness.run_steps(2);
        assert_eq!(text_size(&harness, fixture.text), 48.0, "{change}");
        assert_eq!(harness.state().session.history_len(), history, "{change}");
        assert!(!harness.state().session.gesture_active(), "{change}");
        harness
            .state_mut()
            .set_item_field(fixture.audio, "audio_gain", "3")
            .unwrap();
        assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    }
}

#[test]
fn scrub_clamps_and_typed_drafts_commit_or_cancel() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.text);
    drag(
        &mut harness,
        "inspector.text_size.scrub",
        egui::vec2(10_000.0, 0.0),
    );
    assert_eq!(text_size(&harness, fixture.text), 4096.0);
    drag(
        &mut harness,
        "inspector.text_size.scrub",
        egui::vec2(-10_000.0, 0.0),
    );
    assert_eq!(text_size(&harness, fixture.text), 1.0);
    harness
        .state_mut()
        .activate("timeline.deselect_all")
        .unwrap();
    harness.run_steps(2);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);

    // Click-to-type clamps and commits a single registered command.
    select(&mut harness, fixture.text);
    click(&mut harness, "inspector.text_size");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("64".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(text_size(&harness, fixture.text), 64.0);
    let history = harness.state().session.history_len();

    // Escape discards a typed draft and leaves the value and history unchanged.
    click(&mut harness, "inspector.text_size");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("99".into()));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert_eq!(text_size(&harness, fixture.text), 64.0);
    assert_eq!(harness.state().session.history_len(), history);
    assert!(has_widget(&harness, "inspector.text_size"));

    // Invalid input refuses without changing the item.
    click(&mut harness, "inspector.text_size");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("NaN".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(text_size(&harness, fixture.text), 64.0);
    assert_eq!(harness.state().session.history_len(), history);
    assert!(harness.state().inspect().is_ok());
}

#[test]
fn opacity_and_audio_gain_scrub_through_their_tabs() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.video);
    click(&mut harness, "inspector.tab.blending");
    let history = harness.state().session.history_len();
    drag(
        &mut harness,
        "inspector.opacity.scrub",
        egui::vec2(-200.0, 0.0),
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    let path = project::AnimationPath::new("opacity").unwrap();
    let value = item(&harness, fixture.video)
        .animations
        .value_at(&path, Tick::ZERO)
        .unwrap();
    let Some(project::ParamValue::Scalar(value)) = value else {
        panic!("opacity scalar")
    };
    assert!((value - 0.5).abs() < 1e-9, "{value}");

    select(&mut harness, fixture.audio);
    let history = harness.state().session.history_len();
    drag(
        &mut harness,
        "inspector.audio_gain.scrub",
        egui::vec2(16.0, 0.0),
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    let ItemParams::Audio(params) = &item(&harness, fixture.audio).content else {
        panic!("audio")
    };
    assert_eq!(params.gain_db, 4.0);
}

#[test]
fn assets_tabs_switch_views_and_persist_with_the_app_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let mut harness = build(fixture().editor);
    harness.run_steps(3);
    assert!(has_widget(&harness, "assets.tab.media"));
    assert!(has_widget(&harness, "file.import"));
    click(&mut harness, "assets.tab.sounds");
    assert_eq!(harness.state().preferences.assets_tab, AssetsTab::Sounds);
    assert!(!has_widget(&harness, "file.import"));
    click(&mut harness, "assets.tab.media");
    assert_eq!(harness.state().preferences.assets_tab, AssetsTab::Media);
    assert!(has_widget(&harness, "file.import"));
    click(&mut harness, "assets.tab.captions");
    store.save(&harness.state().preferences).unwrap();
    let restored = store.load().unwrap();
    assert_eq!(restored.assets_tab, AssetsTab::Captions);
    let mut restarted = build(fixture().editor);
    restarted.state_mut().preferences = restored;
    restarted.run_steps(3);
    assert_eq!(
        restarted.state().preferences.assets_tab,
        AssetsTab::Captions
    );
    assert!(!has_widget(&restarted, "file.import"));
}

#[test]
fn active_tab_memory_round_trips_through_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let first = fixture();
    let image = first.image;
    let mut harness = build(first.editor);
    harness.run_steps(3);
    select(&mut harness, image);
    click(&mut harness, "inspector.tab.masks");
    store.save(&harness.state().preferences).unwrap();
    let restored = store.load().unwrap();
    assert_eq!(
        restored.properties_tabs.get("image").map(String::as_str),
        Some("masks")
    );
    let restarted_fixture = fixture();
    let restarted_image = restarted_fixture.image;
    let mut restarted = build(restarted_fixture.editor);
    restarted.state_mut().preferences = restored;
    restarted.run_steps(3);
    select(&mut restarted, restarted_image);
    assert!(has_widget(&restarted, "inspector.tab.masks"));
    assert_eq!(
        restarted
            .state()
            .preferences
            .properties_tabs
            .get("image")
            .map(String::as_str),
        Some("masks")
    );
}

fn text_size(harness: &Harness<Editor>, id: ItemId) -> f32 {
    let ItemParams::Text(params) = &item(harness, id).content else {
        panic!("text")
    };
    params.font_size
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-06-panels.png"]
fn panel_tabs_render_a_representative_editor() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.video);
    click(&mut harness, "inspector.tab.blending");
    click(&mut harness, "assets.tab.effects");
    let image = harness.render().expect("offscreen render");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-06-panels.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}
