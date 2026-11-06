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
    effect_previews::EffectPreviewStatus,
    egui,
    playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;

struct Fixture {
    editor: Editor,
    video: ItemId,
    image: ItemId,
    graphic: ItemId,
    audio: ItemId,
    main: TrackId,
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
        path: std::env::temp_dir().join(format!("vibecut-effects-{id}.mp4")),
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
    duration: Tick,
    asset: Option<AssetId>,
    content: ItemParams,
) -> ItemId {
    let outcome = session
        .apply(Command::InsertItem {
            placement: Placement::Track { track },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: asset,
                source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
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

fn add_effect(session: &mut Session, item: ItemId, id: &str, enabled: bool) {
    session
        .execute(
            "effects.add",
            serde_json::json!({
                "item": item,
                "effect": {
                    "id": id,
                    "enabled": enabled,
                    "params": {"kind": "blur", "intensity": 15.0}
                }
            }),
        )
        .unwrap();
}

fn fixture() -> Fixture {
    let mut session =
        Session::new(Project::empty(ProjectId(9), "Effects".into()).unwrap(), 200).unwrap();
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
    ] {
        session
            .apply(Command::AddAsset {
                asset: asset(id, streams),
            })
            .unwrap();
    }
    let main = session.project().data().scene().timeline.main.id;
    let audio_track = add_track(&mut session, "Audio", TrackKind::Audio);
    let video = insert(
        &mut session,
        main,
        Tick::ZERO,
        Tick(2 * SECOND),
        Some(AssetId(1)),
        ItemParams::Video(project::VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
        }),
    );
    let image = insert(
        &mut session,
        main,
        Tick(2 * SECOND),
        Tick(SECOND),
        Some(AssetId(3)),
        ItemParams::Video(project::VideoParams {
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
        }),
    );
    let graphic = insert(
        &mut session,
        main,
        Tick(3 * SECOND),
        Tick(SECOND),
        None,
        ItemParams::Overlay(project::OverlayParams {
            color: Color([10, 20, 30, 255]),
        }),
    );
    let audio = insert(
        &mut session,
        audio_track,
        Tick::ZERO,
        Tick(2 * SECOND),
        Some(AssetId(2)),
        ItemParams::Audio(project::AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
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
        video,
        image,
        graphic,
        audio,
        main,
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

fn effects(harness: &Harness<Editor>, id: ItemId) -> Vec<project::EffectInstance> {
    item(harness, id).effects
}

fn effect_ids(harness: &Harness<Editor>, id: ItemId) -> Vec<String> {
    effects(harness, id)
        .into_iter()
        .map(|effect| effect.id)
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

fn open_effect_tabs(harness: &mut Harness<Editor>, item: ItemId) {
    select(harness, item);
    click(harness, "assets.tab.effects");
    click(harness, "inspector.tab.effects");
}

fn inject_previews(
    harness: &mut Harness<Editor>,
    adapter: Arc<dyn render::effects::EffectCompositor>,
) {
    harness.state_mut().set_effects(adapter).unwrap();
}

fn wait_preview(
    harness: &mut Harness<Editor>,
    kind: &str,
    wanted: EffectPreviewStatus,
) -> EffectPreviewStatus {
    for _ in 0..400 {
        harness.run_steps(1);
        let Some(status) = harness
            .state()
            .inspect()
            .unwrap()
            .effect_previews
            .get(kind)
            .copied()
        else {
            std::thread::sleep(std::time::Duration::from_millis(2));
            continue;
        };
        if status == wanted {
            return status;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("preview {kind} did not become {wanted:?}");
}

#[test]
fn effects_tab_is_available_for_visual_clip_kinds_only() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    for item in [fixture.video, fixture.image, fixture.graphic] {
        select(&mut harness, item);
        assert!(has_widget(&harness, "inspector.tab.effects"), "{item:?}");
        click(&mut harness, "inspector.tab.effects");
        assert!(has_widget(&harness, "effects.stack.empty"), "{item:?}");
    }
    select(&mut harness, fixture.audio);
    assert!(!has_widget(&harness, "inspector.tab.effects"));
}

#[test]
fn effect_tile_drop_adds_to_a_clip_and_refuses_audio_with_a_reason() {
    let fixture = fixture();
    let video = fixture.video;
    let audio = fixture.audio;
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.tile.blur").center();
    let target = widget_rect(&harness, &format!("timeline.item.{}", video.0));
    drag_to(&mut harness, "effects.tile.blur", target.center() - start);
    harness.drop_at(target.center());
    harness.run_steps(3);
    assert_eq!(effect_ids(&harness, video), vec!["blur".to_owned()]);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert_eq!(harness.state().selection, [video].into_iter().collect());

    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(effects(&harness, video).is_empty());

    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.tile.blur").center();
    let target = widget_rect(&harness, &format!("timeline.item.{}", audio.0));
    drag_to(&mut harness, "effects.tile.blur", target.center() - start);
    harness.drop_at(target.center());
    harness.run_steps(3);
    assert!(effects(&harness, audio).is_empty());
    assert_eq!(harness.state().session.history_len(), history);
    let toast = harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("Effects don't apply to audio"), "{toast}");
}

#[test]
fn effect_drag_escape_and_missed_drop_leave_the_project_untouched() {
    let fixture = fixture();
    let video = fixture.video;
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.tile.blur").center();
    let target = widget_rect(&harness, &format!("timeline.item.{}", video.0));
    drag_to(&mut harness, "effects.tile.blur", target.center() - start);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.drop_at(target.center());
    harness.run_steps(3);
    assert!(effects(&harness, video).is_empty());
    assert_eq!(harness.state().session.history_len(), history);

    let start = widget_rect(&harness, "effects.tile.blur").center();
    let miss = egui::pos2(700.0, 540.0);
    drag_to(&mut harness, "effects.tile.blur", miss - start);
    harness.drop_at(miss);
    harness.run_steps(3);
    assert!(effects(&harness, video).is_empty());
    assert_eq!(harness.state().session.history_len(), history);
    let toast = harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("Drop on a video or image clip"), "{toast}");
}

#[test]
fn effect_plus_adds_to_the_selected_clip_and_needs_a_target() {
    let fixture = fixture();
    let video = fixture.video;
    let image = fixture.image;
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    click(&mut harness, "assets.tab.effects");
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.add.blur");
    assert!(effects(&harness, video).is_empty());
    assert_eq!(harness.state().session.history_len(), history);
    let toast = harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("Select a clip first"), "{toast}");

    harness.state_mut().selection = [video, image].into_iter().collect();
    harness.run_steps(3);
    click(&mut harness, "effects.add.blur");
    assert!(effects(&harness, video).is_empty());
    assert!(effects(&harness, image).is_empty());
    assert_eq!(harness.state().session.history_len(), history);

    select(&mut harness, image);
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.add.blur");
    assert_eq!(effect_ids(&harness, image), vec!["blur".to_owned()]);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert_eq!(harness.state().selection, [image].into_iter().collect());
}

#[test]
fn effect_headers_reorder_toggle_and_remove_through_commands() {
    let fixture = fixture();
    let video = fixture.video;
    let image = fixture.image;
    let mut editor = fixture.editor;
    add_effect(&mut editor.session, video, "blur", true);
    add_effect(&mut editor.session, video, "blur2", true);
    add_effect(&mut editor.session, image, "blur", true);
    add_effect(&mut editor.session, image, "blur2", true);
    editor
        .preferences
        .properties_tabs
        .insert("image".into(), "effects".into());
    let mut harness = build(editor);
    harness.run_steps(3);
    open_effect_tabs(&mut harness, video);
    assert_eq!(effect_ids(&harness, video), vec!["blur", "blur2"]);

    for (source, target, cancel, changed) in [
        ("blur2", "blur", false, true),
        ("blur", "blur2", false, true),
        ("blur", "blur", false, false),
        ("blur2", "blur", true, false),
    ] {
        let history = harness.state().session.history_len();
        let handle = format!("effects.row.{source}.handle");
        let start = widget_rect(&harness, &handle).center();
        let target = widget_rect(&harness, &format!("effects.toggle.{target}")).center();
        drag_to(&mut harness, &handle, target - start);
        if cancel {
            harness.key_press(egui::Key::Escape);
            harness.run_steps(2);
        }
        harness.drop_at(target);
        harness.run_steps(3);
        assert_eq!(
            effect_ids(&harness, video),
            if changed {
                vec!["blur2", "blur"]
            } else {
                vec!["blur", "blur2"]
            },
            "source {source}, cancel {cancel}"
        );
        assert_eq!(
            harness.state().session.history_len().0,
            history.0 + usize::from(changed)
        );
        if changed {
            harness.state_mut().activate("edit.undo").unwrap();
            harness.run_steps(2);
        }
        assert_eq!(effect_ids(&harness, video), vec!["blur", "blur2"]);
    }

    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.row.blur2.handle").center();
    let target = widget_rect(&harness, "effects.toggle.blur").center();
    drag_to(&mut harness, "effects.row.blur2.handle", target - start);
    select(&mut harness, image);
    let target = widget_rect(&harness, "effects.toggle.blur").center();
    harness.drop_at(target);
    harness.run_steps(3);
    assert_eq!(effect_ids(&harness, video), vec!["blur", "blur2"]);
    assert_eq!(effect_ids(&harness, image), vec!["blur", "blur2"]);
    assert_eq!(harness.state().session.history_len(), history);
    select(&mut harness, video);

    let history = harness.state().session.history_len();
    click(&mut harness, "effects.toggle.blur");
    assert!(!effects(&harness, video)[0].enabled);
    assert!(has_widget(&harness, "effects.row.blur.disabled"));
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    click(&mut harness, "effects.toggle.blur");
    assert!(effects(&harness, video)[0].enabled);

    let history = harness.state().session.history_len();
    click(&mut harness, "effects.remove.blur2");
    assert_eq!(effect_ids(&harness, video), vec!["blur"]);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(effect_ids(&harness, video), vec!["blur", "blur2"]);
}

#[test]
fn effect_param_scrub_reverses_cancels_and_commits_once() {
    let fixture = fixture();
    let video = fixture.video;
    let mut editor = fixture.editor;
    add_effect(&mut editor.session, video, "blur", true);
    let mut harness = build(editor);
    harness.run_steps(3);
    open_effect_tabs(&mut harness, video);
    let intensity = |harness: &Harness<Editor>| match effects(harness, video)[0].params {
        project::EffectParams::Blur { intensity } => intensity,
    };

    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.blur.param.intensity.scrub").center();
    drag_to(
        &mut harness,
        "effects.blur.param.intensity.scrub",
        egui::vec2(40.0, 0.0),
    );
    assert!(harness.state().session.gesture_active());
    assert_eq!(harness.state().session.history_len(), history);
    assert_eq!(intensity(&harness), 35.0);
    harness.hover_at(start);
    harness.run_steps(2);
    harness.drop_at(start);
    harness.run_steps(2);
    assert_eq!(intensity(&harness), 15.0, "reversal leaves the origin");
    assert_eq!(harness.state().session.history_len(), history);

    drag_to(
        &mut harness,
        "effects.blur.param.intensity.scrub",
        egui::vec2(40.0, 0.0),
    );
    assert_eq!(intensity(&harness), 35.0);
    harness.drop_at(start + egui::vec2(40.0, 0.0));
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(intensity(&harness), 15.0);

    let history = harness.state().session.history_len();
    drag_to(
        &mut harness,
        "effects.blur.param.intensity.scrub",
        egui::vec2(40.0, 0.0),
    );
    assert_eq!(intensity(&harness), 35.0);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.drop_at(start + egui::vec2(40.0, 0.0));
    harness.run_steps(2);
    assert_eq!(intensity(&harness), 15.0);
    assert_eq!(harness.state().session.history_len(), history);

    click(&mut harness, "effects.blur.param.intensity");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("150".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(intensity(&harness), 100.0, "typed values clamp");
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.blur.param.intensity");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("NaN".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(intensity(&harness), 100.0);
    assert_eq!(harness.state().session.history_len(), history);
}

#[test]
fn effect_add_refuses_locked_tracks_and_the_sixty_fifth_effect() {
    let locked = fixture();
    let video = locked.video;
    let main = locked.main;
    let mut editor = locked.editor;
    editor
        .session
        .execute(
            "track.set_flags",
            serde_json::json!({"track": main, "muted": false, "hidden": false, "locked": true}),
        )
        .unwrap();
    let mut harness = build(editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.add.blur");
    assert!(effects(&harness, video).is_empty());
    assert_eq!(harness.state().session.history_len(), history);
    let toast = harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("Track is locked"), "{toast}");

    let limit = fixture();
    let video = limit.video;
    let mut editor = limit.editor;
    for index in 0..64 {
        add_effect(&mut editor.session, video, &format!("fx{index}"), true);
    }
    let mut harness = build(editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    let before = effects(&harness, video).len();
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.add.blur");
    assert_eq!(effects(&harness, video).len(), before);
    assert_eq!(harness.state().session.history_len(), history);
    let toast = harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("Effect limit reached"), "{toast}");
}

struct BlockingPreview;
impl render::effects::EffectCompositor for BlockingPreview {
    fn apply(
        &self,
        _: &render::Image,
        _: &[render::effects::EffectPass],
        _: render::effects::EffectInputs<'_>,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<render::Image> {
        while !hooks.cancelled() {
            std::thread::yield_now();
        }
        Err(render::Error::Cancelled)
    }
}
struct FailingPreview;
impl render::effects::EffectCompositor for FailingPreview {
    fn apply(
        &self,
        _: &render::Image,
        _: &[render::effects::EffectPass],
        _: render::effects::EffectInputs<'_>,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<render::Image> {
        Err(render::Error::Unsupported("test preview"))
    }
}

#[test]
fn effect_preview_tiles_render_pending_failed_and_ready_states() {
    let pending = fixture();
    let video = pending.video;
    let mut harness = build(pending.editor);
    inject_previews(&mut harness, Arc::new(BlockingPreview));
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    assert_eq!(
        harness
            .state()
            .inspect()
            .unwrap()
            .effect_previews
            .get("blur")
            .copied(),
        Some(EffectPreviewStatus::Pending)
    );
    assert!(has_widget(&harness, "effects.tile.blur.pending"));

    let failed = fixture();
    let video = failed.video;
    let mut harness = build(failed.editor);
    inject_previews(&mut harness, Arc::new(FailingPreview));
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    wait_preview(&mut harness, "blur", EffectPreviewStatus::Failed);
    assert!(has_widget(&harness, "effects.tile.blur.failed"));
    assert!(has_widget(&harness, "effects.preview.retry.blur"));

    let ready = fixture();
    let video = ready.video;
    let mut harness = build(ready.editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    wait_preview(&mut harness, "blur", EffectPreviewStatus::Ready);
    assert!(has_widget(&harness, "effects.tile.blur.ready"));
    assert_eq!(
        harness
            .state()
            .inspect()
            .unwrap()
            .effect_previews
            .get("blur")
            .copied(),
        Some(EffectPreviewStatus::Ready)
    );
}

#[test]
fn effect_preferences_round_trip_through_the_app_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    click(&mut harness, "assets.tab.effects");
    store.save(&harness.state().preferences).unwrap();
    let restored = store.load().unwrap();
    assert_eq!(restored.assets_tab, vibecut::panels::AssetsTab::Effects);
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-33-effects.png"]
fn effects_panel_renders_tiles_stack_and_preview() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    for (scale, filename) in [(1.0, "p1-33-effects.png"), (2.0, "p1-33-effects-2x.png")] {
        let fixture = fixture();
        let video = fixture.video;
        let mut editor = fixture.editor;
        add_effect(&mut editor.session, video, "blur", true);
        add_effect(&mut editor.session, video, "blur2", false);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_pixels_per_point(scale)
            .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
        harness.run_steps(3);
        open_effect_tabs(&mut harness, video);
        wait_preview(&mut harness, "blur", EffectPreviewStatus::Ready);
        let image = harness.render().expect("offscreen render");
        image.save(target.join(filename)).unwrap();
        assert!(image.width() > 0 && image.height() > 0);
    }
}
