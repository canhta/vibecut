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
use engine::control::{FontEntry, FontLoad, FontLoadState, FontOrigin, FontState, JobView};
use engine::jobs::{JobId, JobKind, JobStage, JobState};
use engine::{ByteBudget, Command, CreatedId, ItemParamPatch, NewItem, Placement, Session};
use media::{AssetId, ContentHash, SourceAsset, SourceStream};
use project::{Color, ItemId, ItemParams, Project, ProjectId, TextBackground, TrackId, TrackKind};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::lifecycle::EditorSession::Memory;
use vibecut::{
    Editor, ElementKind, PropertiesTab, egui,
    font_picker::{Backend, Status},
    panels::AssetsTab,
    preferences::{FontTab, PreferenceStore, Preferences},
    services::playback::{Files, Playback},
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
    let mut builder = testkit::source::stream(kind).duration(Tick(10 * SECOND));
    if let Some(frame_count) = frame_count {
        builder = builder.frame_count(frame_count);
    }
    builder.build()
}

fn asset(id: u64, mut streams: Vec<SourceStream>) -> SourceAsset {
    for (index, stream) in streams.iter_mut().enumerate() {
        stream.index = index;
    }
    testkit::source::asset(
        id,
        std::env::temp_dir().join(format!("vibecut-panels-{id}.mp4")),
    )
    .format("mov,mp4")
    .duration(Tick(10 * SECOND))
    .content_hash(ContentHash::from_bytes(&[id as u8; 8]))
    .streams(streams)
    .build()
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
                retime: Default::default(),
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
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
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
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            crop: None,
        }),
    );
    let silent_video = insert(
        &mut session,
        main,
        Tick(4 * SECOND),
        Tick(SECOND),
        Some(AssetId(4)),
        ItemParams::Video(project::VideoParams {
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            crop: None,
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
            ..Default::default()
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
        text,
        video,
        silent_video,
        image,
        graphic,
        audio,
    }
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn select(harness: &mut Harness<Editor>, item: ItemId) {
    harness.state_mut().selection = [item].into_iter().collect();
    harness.run_steps(3);
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    if matches!(
        id,
        "inspector.text_color.picker" | "inspector.background_color"
    ) || id.starts_with("inspector.text_")
        || id.starts_with("text.animation.")
    {
        for _ in 0..12 {
            let panel = harness.state().panels()["inspector"];
            let top = widget_rect(harness, "inspector.tab.text").bottom() + 8.0;
            let viewport = egui::Rect::from_min_max(
                egui::pos2(panel.left() + 12.0, top),
                panel.max - egui::vec2(12.0, 12.0),
            );
            let target = widget_rect(harness, id);
            if viewport.contains_rect(target) {
                break;
            }
            harness.hover_at(viewport.center());
            harness.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, if target.top() < top { 120.0 } else { -120.0 }),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run_steps(8);
        }
    }
    if matches!(id, "inspector.tab.transform" | "inspector.tab.blending")
        && !has_widget(harness, id)
    {
        click(harness, "inspector.group.basic");
    }
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

fn enter(harness: &mut Harness<Editor>, id: &str, value: &str) {
    click(harness, id);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(value.to_owned()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
}

#[test]
fn transform_fields_clamp_reset_scrub_cancel_and_persist() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.video);
    let before = harness.state().session.project().clone();
    enter(&mut harness, "inspector.transform.rotate", "999");
    assert_eq!(
        project::TransformField::Rotate
            .value_at(&item(&harness, fixture.video).animations, Tick::ZERO)
            .unwrap(),
        360.0
    );
    let bytes = project::encode(harness.state().session.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert_eq!(loaded.project, *harness.state().session.project().as_ref());
    click(&mut harness, "inspector.section.transform.reset");
    assert_eq!(
        project::TransformField::Rotate
            .value_at(&item(&harness, fixture.video).animations, Tick::ZERO)
            .unwrap(),
        0.0
    );
    click(&mut harness, "inspector.transform.rotate");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("42".to_owned()));
    harness.run_steps(2);
    click(&mut harness, "inspector.section.transform.reset");
    assert_eq!(
        project::TransformField::Rotate
            .value_at(&item(&harness, fixture.video).animations, Tick::ZERO)
            .unwrap(),
        0.0
    );
    enter(&mut harness, "inspector.transform.scaleX", "-4");
    assert_eq!(
        project::TransformField::ScaleX
            .value_at(&item(&harness, fixture.video).animations, Tick::ZERO)
            .unwrap(),
        0.01
    );
    let prior = harness.state().session.project().clone();
    drag_to(
        &mut harness,
        "inspector.transform.positionX.scrub",
        egui::vec2(44.0, 0.0),
    );
    assert!(harness.state().session.gesture_active());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().session.project(), &prior);
    assert_ne!(harness.state().session.project(), &before);
}

#[test]
fn scale_is_uniform_until_toggled_and_steppers_move_one_unit() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.video);
    let value = |harness: &Harness<Editor>, field: project::TransformField| {
        field
            .value_at(&item(harness, fixture.video).animations, Tick::ZERO)
            .unwrap()
    };
    use project::TransformField as F;
    // (field id, entered text, expected scaleX, expected scaleY, uniform toggle first)
    for (id, text, x, y, toggle) in [
        ("inspector.transform.scaleX", "200", 2.0, 2.0, false),
        ("inspector.transform.scaleX", "50", 0.5, 0.5, false),
        ("inspector.transform.scaleY", "150", 0.5, 1.5, true),
    ] {
        if toggle {
            click(&mut harness, "inspector.uniform_scale");
        }
        enter(&mut harness, id, text);
        assert_eq!(value(&harness, F::ScaleX), x, "{id} {text}");
        assert_eq!(value(&harness, F::ScaleY), y, "{id} {text}");
    }
    click(&mut harness, "inspector.transform.positionX.increase");
    assert_eq!(value(&harness, F::PositionX), 1.0);
    click(&mut harness, "inspector.transform.positionX.decrease");
    click(&mut harness, "inspector.transform.positionX.decrease");
    assert_eq!(value(&harness, F::PositionX), -1.0);
    click(&mut harness, "inspector.section.transform.reset");
    assert_eq!(value(&harness, F::ScaleY), 1.0);
    assert_eq!(value(&harness, F::PositionX), 0.0);
}

#[test]
fn color_picker_is_a_popover_left_of_the_properties_panel() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.graphic);
    click(&mut harness, "inspector.graphic_fill");
    let panel = harness.state().panels()["inspector"];
    let picker = harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == "color.hex")
        .expect("picker open")
        .rect;
    assert!(
        picker.right() <= panel.left(),
        "{picker:?} covers {panel:?}"
    );
}

#[test]
fn color_drag_isolated_until_release_one_undo_and_escape_cancels() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.text);
    click(&mut harness, "inspector.text_color.picker");
    harness.run_steps(3);
    let before = harness.state().session.project().clone();
    let slider = widget_rect(&harness, "color.v.slider");
    harness.hover_at(slider.center());
    harness.drag_at(slider.center());
    harness.run_steps(2);
    harness.hover_at(slider.left_center() + egui::vec2(15.0, 0.0));
    harness.run_steps(2);
    assert_eq!(
        harness.state().session.project(),
        &before,
        "preview must not mutate editable snapshot"
    );
    assert_ne!(
        harness.state().inspect().unwrap().color_preview,
        Some(Color([255; 4]))
    );
    harness.drop_at(slider.left_center() + egui::vec2(15.0, 0.0));
    harness.run_steps(2);
    assert_ne!(harness.state().session.project(), &before);
    harness.state_mut().session.undo().unwrap();
    assert_eq!(harness.state().session.project(), &before);
    harness.run_steps(2);
    click(&mut harness, "inspector.text_color.picker");
    harness.run_steps(2);
    let slider = widget_rect(&harness, "color.v.slider");
    harness.drag_at(slider.center());
    harness.run_steps(2);
    harness.hover_at(slider.left_center());
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.drop_at(slider.left_center());
    harness.run_steps(2);
    assert_eq!(harness.state().session.project(), &before);
    assert_eq!(harness.state().inspect().unwrap().color_preview, None);
    let history = harness.state().session.history_len();
    let recents = harness.state().preferences.recent_colors.clone();
    click(&mut harness, "inspector.text_color.picker");
    harness.run_steps(2);
    let slider = widget_rect(&harness, "color.v.slider");
    harness.drag_at(slider.center());
    harness.run_steps(2);
    harness.hover_at(slider.left_center());
    harness.run_steps(2);
    harness.hover_at(slider.right_center());
    harness.run_steps(2);
    harness.drop_at(slider.right_center());
    harness.run_steps(2);
    assert_eq!(harness.state().session.project(), &before);
    assert_eq!(harness.state().session.history_len(), history);
    assert_eq!(harness.state().preferences.recent_colors, recents);
}

#[test]
fn picker_hex_hsv_recents_unavailable_and_screenshot() {
    #[cfg(not(target_os = "macos"))]
    {
        let mut native = vibecut::services::color_picker::NativeEyedropper::default();
        assert_eq!(
            native.state,
            vibecut::services::color_picker::EyedropperState::Unavailable
        );
        assert!(matches!(
            native.start(),
            Err(vibecut::Error::Unsupported(_))
        ));
        assert_eq!(native.poll().unwrap(), None);
    }
    let mut fixture = fixture();
    // A screenshot needs a healthy generated preview, not the offline media
    // deliberately retained by the general panel fixture.
    fixture
        .editor
        .session
        .apply(Command::Delete {
            items: vec![
                fixture.video,
                fixture.silent_video,
                fixture.image,
                fixture.audio,
            ],
        })
        .unwrap();
    fixture
        .editor
        .playback
        .seek(Tick::ZERO, false, fixture.editor.session.project().clone())
        .unwrap();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.text);
    click(&mut harness, "inspector.text_color.picker");
    harness.run_steps(3);
    enter(&mut harness, "color.hex", "#12AB3480");
    assert!(
        matches!(item(&harness,fixture.text).content,ItemParams::Text(p) if p.color==Color([18,171,52,128]))
    );
    enter(&mut harness, "color.h", "240");
    enter(&mut harness, "color.s", "100");
    enter(&mut harness, "color.v", "100");
    enter(&mut harness, "color.a", "50");
    assert!(
        matches!(item(&harness,fixture.text).content,ItemParams::Text(p) if p.color==Color([0,0,255,128]))
    );
    harness
        .state_mut()
        .set_eyedropper(vibecut::services::color_picker::NativeEyedropper::unavailable())
        .unwrap();
    click(&mut harness, "color.eyedropper");
    assert_eq!(
        harness.state().inspect().unwrap().eyedropper,
        Some(vibecut::services::color_picker::EyedropperState::Unavailable)
    );
    assert!(has_widget(&harness, "color.eyedropper.status"));
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("prefs.json"));
    store.save(&harness.state().preferences).unwrap();
    assert_eq!(
        store.load().unwrap().recent_colors,
        harness.state().preferences.recent_colors
    );
    assert_eq!(
        harness.state().preferences.recent_colors.first(),
        Some(&Color([0, 0, 255, 128]))
    );
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-45-colour.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    harness.render().unwrap().save(&path).unwrap();
    let swatch = vibecut::services::color_picker::palette::solid(5).unwrap();
    click(&mut harness, "color.swatch.5");
    assert!(matches!(item(&harness,fixture.text).content,ItemParams::Text(p) if p.color==swatch));
    click(&mut harness, "color.recent.1");
    assert!(
        matches!(item(&harness,fixture.text).content,ItemParams::Text(p) if p.color==Color([0,0,255,128]))
    );
}

#[test]
fn shared_picker_edits_background_graphic_and_canvas_through_commands() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    for (selection, button) in [
        (Some(fixture.text), "inspector.text_color.picker"),
        (Some(fixture.text), "inspector.background_color"),
        (Some(fixture.graphic), "inspector.graphic_fill"),
        (Some(fixture.graphic), "inspector.graphic_stroke"),
        (None, "inspector.canvas_background"),
    ] {
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        harness.state_mut().selection = selection.into_iter().collect();
        harness.run_steps(3);
        if button == "inspector.background_color" {
            harness
                .state_mut()
                .session
                .apply(Command::SetItemField {
                    item: fixture.text,
                    patch: ItemParamPatch::TextBackground(Some(TextBackground {
                        color: Color([1, 2, 3, 4]),
                        padding_x: 17.0,
                        padding_y: 15.0,
                        offset_x: 0.0,
                        offset_y: 0.0,
                        radius: 9.0,
                    })),
                })
                .unwrap();
            harness.run_steps(2);
        }
        let before = harness.state().session.project().clone();
        click(&mut harness, button);
        harness.run_steps(3);
        enter(&mut harness, "color.hex", "#AABBCC80");
        assert_ne!(harness.state().session.project(), &before, "{button}");
        let chosen = Color([170, 187, 204, 128]);
        match button {
            "inspector.text_color.picker" => assert!(
                matches!(item(&harness,fixture.text).content,ItemParams::Text(p) if p.color==chosen)
            ),
            "inspector.background_color" => {
                let ItemParams::Text(p) = item(&harness, fixture.text).content else {
                    panic!("text colour target")
                };
                let background = p.background.unwrap();
                assert_eq!(background.color, chosen);
                assert_eq!((background.padding_x, background.padding_y), (17.0, 15.0));
                assert_eq!(background.radius, 9.0);
            }
            "inspector.graphic_fill" => assert!(
                matches!(item(&harness,fixture.graphic).content,ItemParams::Overlay(p) if p.color==chosen)
            ),
            "inspector.graphic_stroke" => assert_eq!(
                render::color::at(
                    &item(&harness, fixture.graphic).animations,
                    &project::AnimationPath::new("params.strokeColor").unwrap(),
                    Tick::ZERO,
                    Color([0; 4])
                )
                .unwrap(),
                chosen
            ),
            _ => assert_eq!(
                harness.state().session.project().data().canvas.background,
                chosen
            ),
        }
        // Every target shares bounded entry, isolated drafts and one-step publication.
        for (field, value, expected) in [
            ("color.h", "999", Color([204, 170, 170, 128])),
            ("color.s", "-1", Color([204, 204, 204, 128])),
            ("color.v", "-1", Color([0, 0, 0, 128])),
            ("color.a", "999", Color([0, 0, 0, 255])),
        ] {
            enter(&mut harness, field, value);
            assert_eq!(
                harness.state().inspect().unwrap().color_preview,
                Some(expected),
                "{button}: {field}"
            );
        }
        let committed = harness.state().session.project().clone();
        let history = harness.state().session.history_len();
        let recents = harness.state().preferences.recent_colors.clone();
        let slider = widget_rect(&harness, "color.a.slider");
        harness.drag_at(slider.right_center());
        harness.run_steps(2);
        harness.hover_at(slider.left_center());
        harness.run_steps(2);
        assert_eq!(harness.state().session.project(), &committed, "{button}");
        assert_eq!(harness.state().session.history_len(), history, "{button}");
        assert_eq!(
            harness.state().inspect().unwrap().color_preview,
            Some(Color([0; 4])),
            "{button}"
        );
        harness.hover_at(slider.right_center());
        harness.run_steps(2);
        harness.drop_at(slider.right_center());
        harness.run_steps(2);
        assert_eq!(harness.state().session.project(), &committed, "{button}");
        assert_eq!(harness.state().session.history_len(), history, "{button}");
        assert_eq!(
            harness.state().preferences.recent_colors,
            recents,
            "{button}"
        );
        harness.drag_at(slider.right_center());
        harness.run_steps(2);
        harness.hover_at(slider.left_center());
        harness.run_steps(2);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        harness.drop_at(slider.left_center());
        harness.run_steps(2);
        assert_eq!(harness.state().session.project(), &committed, "{button}");
        assert_eq!(harness.state().session.history_len(), history, "{button}");
        assert_eq!(
            harness.state().inspect().unwrap().color_preview,
            None,
            "{button}"
        );
        assert_eq!(
            harness.state().preferences.recent_colors,
            recents,
            "{button}"
        );
        click(&mut harness, button);
        let slider = widget_rect(&harness, "color.a.slider");
        harness.drag_at(slider.right_center());
        harness.run_steps(2);
        harness.hover_at(slider.left_center());
        harness.run_steps(2);
        harness.drop_at(slider.left_center());
        harness.run_steps(2);
        assert_ne!(harness.state().session.project(), &committed, "{button}");
        assert_eq!(
            harness.state().session.history_len().0,
            history.0 + 1,
            "{button}"
        );
        harness.state_mut().session.undo().unwrap();
        assert_eq!(harness.state().session.project(), &committed, "{button}");
        // Undo the four bounds edits and the initial hex pick.
        for _ in 0..5 {
            harness.state_mut().session.undo().unwrap();
        }
        assert_eq!(harness.state().session.project(), &before, "{button}");
    }
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
        if let Some(field) = active_field {
            assert!(has_widget(&harness, field), "{item:?} default tab content");
        }
        // Transform and blending share one group tab with sub-tabs.
        if has_widget(&harness, "inspector.group.basic") {
            click(&mut harness, "inspector.group.basic");
        }
        let mut seen = visible_tabs(&harness);
        seen.sort();
        let mut want = names(expected);
        want.sort();
        assert_eq!(seen, want, "{item:?}");
    }
    // A video with no audio stream hides its Audio tab.
    select(&mut harness, fixture.silent_video);
    let expected: Vec<_> = ElementKind::Video
        .tabs()
        .iter()
        .copied()
        .filter(|tab| *tab != PropertiesTab::Audio)
        .collect();
    let mut seen = visible_tabs(&harness);
    seen.sort();
    let mut want = names(&expected);
    want.sort();
    assert_eq!(seen, want);
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
                PropertiesTab::Effects,
            ][..],
        ),
        (
            ElementKind::Video,
            &[
                PropertiesTab::Transform,
                PropertiesTab::Audio,
                PropertiesTab::Speed,
                PropertiesTab::Blending,
                PropertiesTab::Adjust,
                PropertiesTab::RemoveBackground,
                PropertiesTab::Effects,
            ][..],
        ),
        (
            ElementKind::Image,
            &[
                PropertiesTab::Transform,
                PropertiesTab::Blending,
                PropertiesTab::Adjust,
                PropertiesTab::RemoveBackground,
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
    // The strip lists the text tab, the basic group (its transform sub-tab is active) and effects.
    assert_eq!(
        visible_tabs(&harness),
        [
            "inspector.tab.text",
            "inspector.tab.effects",
            "inspector.tab.transform",
            "inspector.tab.blending"
        ],
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
fn mask_row_opens_every_capture_shape_and_none_removes_one_command_step_each() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.video);
    assert!(has_widget(&harness, "inspector.mask.row"));
    assert!(mask_of(&harness, fixture.video).is_none());

    click(&mut harness, "inspector.mask.row");
    assert!(has_widget(&harness, "inspector.mask.shape.none"));
    for (choice, shape) in [
        ("horizontal", project::MaskShape::Split),
        ("mirror", project::MaskShape::CinematicBars),
        ("circle", project::MaskShape::Ellipse),
        ("rectangle", project::MaskShape::Rectangle),
        ("heart", project::MaskShape::Heart),
        ("stars", project::MaskShape::Star),
    ] {
        let history = harness.state().session.history_len();
        click(&mut harness, &format!("inspector.mask.shape.{choice}"));
        assert_eq!(
            harness.state().session.history_len().0,
            history.0 + 1,
            "{choice} is one undo step"
        );
        let mask = mask_of(&harness, fixture.video).expect("the choice adds a mask");
        assert_eq!(mask.id, "mask1", "the panel keeps one mask per clip");
        assert_eq!(mask.shape, shape, "{choice}");
        assert_eq!(
            mask.rotation, -90.0,
            "{choice} keeps the split's default quarter turn"
        );
    }
    // Geometry is shared: a shape change keeps centre, size and rotation.
    enter(&mut harness, "inspector.mask.size.w", "70");
    enter(&mut harness, "inspector.mask.size.h", "80");
    enter(&mut harness, "inspector.mask.position.x", "10");
    enter(&mut harness, "inspector.mask.position.y", "-20");
    enter(&mut harness, "inspector.mask.rotation", "45");
    let before = mask_of(&harness, fixture.video).unwrap();
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.mask.shape.heart");
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    let after = mask_of(&harness, fixture.video).unwrap();
    assert_eq!(after.shape, project::MaskShape::Heart);
    assert_eq!(
        (
            after.center_x,
            after.center_y,
            after.width,
            after.height,
            after.rotation
        ),
        (
            before.center_x,
            before.center_y,
            before.width,
            before.height,
            before.rotation
        ),
        "changing the shape keeps geometry"
    );
    // The same choice is not an edit.
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.mask.shape.heart");
    assert_eq!(harness.state().session.history_len(), history);
    // None removes the one mask.
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.mask.shape.none");
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert!(mask_of(&harness, fixture.video).is_none());
    // Done closes the panel.
    click(&mut harness, "inspector.mask.done");
    assert!(!has_widget(&harness, "inspector.mask.shape.none"));
}

#[test]
fn mask_fields_are_one_command_step_each_and_round_trip() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.video);
    click(&mut harness, "inspector.mask.row");
    click(&mut harness, "inspector.mask.shape.rectangle");
    let added = harness.state().session.project().clone();
    for (field, value, read, expected) in [
        ("feather", "50", "feather", 50.0),
        ("size.w", "70", "width", 0.7),
        ("size.h", "80", "height", 0.8),
        ("position.x", "10", "center_x", 0.1),
        ("position.y", "-20", "center_y", -0.2),
        ("rotation", "45", "rotation", 45.0),
    ] {
        let history = harness.state().session.history_len();
        enter(&mut harness, &format!("inspector.mask.{field}"), value);
        assert_eq!(
            harness.state().session.history_len().0,
            history.0 + 1,
            "{field} is one undo step"
        );
        let mask = mask_of(&harness, fixture.video).unwrap();
        let actual = match read {
            "feather" => mask.feather,
            "width" => mask.width,
            "height" => mask.height,
            "center_x" => mask.center_x,
            "center_y" => mask.center_y,
            _ => mask.rotation,
        };
        assert!((actual - expected).abs() < 1e-9, "{field} = {actual}");
    }
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.mask.invert");
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert!(mask_of(&harness, fixture.video).unwrap().inverted);
    // Bounds clamp in the engine's units.
    enter(&mut harness, "inspector.mask.feather", "999");
    assert_eq!(mask_of(&harness, fixture.video).unwrap().feather, 100.0);
    enter(&mut harness, "inspector.mask.size.w", "0");
    assert_eq!(mask_of(&harness, fixture.video).unwrap().width, 0.01);
    enter(&mut harness, "inspector.mask.rotation", "2000000");
    assert_eq!(
        mask_of(&harness, fixture.video).unwrap().rotation,
        1_000_000.0
    );
    // Invalid input refuses without a change.
    let before = harness.state().session.project().clone();
    let history = harness.state().session.history_len();
    enter(&mut harness, "inspector.mask.feather", "nope");
    assert_eq!(harness.state().session.project(), &before);
    assert_eq!(harness.state().session.history_len(), history);
    // The feather rail commits once on release.
    let history = harness.state().session.history_len();
    let slider = widget_rect(&harness, "inspector.mask.feather.slider");
    harness.drag_at(slider.right_center());
    harness.run_steps(2);
    harness.hover_at(slider.left_center());
    harness.run_steps(2);
    harness.drop_at(slider.left_center());
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert_eq!(mask_of(&harness, fixture.video).unwrap().feather, 0.0);
    // Undo/redo walks each field edit, then round-trips the saved project.
    for _ in 0..11 {
        harness.state_mut().session.undo().unwrap();
    }
    assert_eq!(harness.state().session.project(), &added);
    for _ in 0..11 {
        harness.state_mut().session.redo().unwrap();
    }
    let saved = project::encode(harness.state().session.project(), false).unwrap();
    let decoded = project::decode(&saved).unwrap();
    assert_eq!(&decoded.project, harness.state().session.project().as_ref());
    assert_eq!(
        decoded
            .project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|item| item.id == fixture.video)
            .unwrap()
            .masks
            .first()
            .unwrap()
            .feather,
        0.0
    );
}

#[test]
fn mask_row_is_absent_off_video_and_image_and_locked_tracks_are_read_only() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    for absent in [fixture.text, fixture.audio, fixture.graphic] {
        select(&mut harness, absent);
        assert!(
            !has_widget(&harness, "inspector.mask.row"),
            "{absent:?} has no mask row"
        );
    }
    select(&mut harness, fixture.image);
    assert!(has_widget(&harness, "inspector.mask.row"));
    click(&mut harness, "inspector.mask.row");
    click(&mut harness, "inspector.mask.shape.circle");
    assert!(matches!(
        mask_of(&harness, fixture.image).unwrap().shape,
        project::MaskShape::Ellipse
    ));
    click(&mut harness, "inspector.mask.done");

    let track = harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .main
        .id;
    harness
        .state_mut()
        .session
        .apply(Command::SetTrackFlags {
            track,
            muted: false,
            hidden: false,
            locked: true,
        })
        .unwrap();
    select(&mut harness, fixture.video);
    click(&mut harness, "inspector.mask.row");
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.mask.shape.rectangle");
    assert_eq!(harness.state().session.history_len(), history);
    assert!(
        mask_of(&harness, fixture.video).is_none(),
        "a locked track refuses the add"
    );
    click(&mut harness, "inspector.mask.done");
}

#[test]
fn mask_panel_screenshot_is_available() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.video);
    click(&mut harness, "inspector.mask.row");
    click(&mut harness, "inspector.mask.shape.rectangle");
    enter(&mut harness, "inspector.mask.feather", "24");
    let image = harness.render().expect("offscreen render");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-36-mask.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image.save(&path).unwrap();
    assert!(path.exists());
}

fn mask_of(harness: &Harness<Editor>, id: ItemId) -> Option<project::Mask> {
    item(harness, id).masks.first().cloned()
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
    assert_eq!(params.volume, 0.04);
}

#[test]
fn text_fades_click_edit_undo_reopen_and_capture_both_locales() {
    for locale in [vibecut::i18n::Locale::En, vibecut::i18n::Locale::Vi] {
        for scale in [1.0, 2.0] {
            let mut f = fixture();
            f.editor.preferences.locale = locale;
            f.editor
                .session
                .apply(Command::Delete {
                    items: vec![f.video, f.silent_video, f.image, f.graphic, f.audio],
                })
                .unwrap();
            f.editor.prepare_subtitles().unwrap(); // Idle captions with no caption target remain eligible.
            f.editor
                .set_item_field(f.text, "text_content", "Tiếng Việt\nEnglish")
                .unwrap();
            f.editor.set_item_field(f.text, "opacity", "0.6").unwrap();
            let mut h = crate::gpu::builder()
                .with_size(egui::vec2(1440.0, 1000.0))
                .with_pixels_per_point(scale)
                .build_ui_state(|ui, e: &mut Editor| e.ui(ui), f.editor);
            h.run_steps(3);
            select(&mut h, f.text);
            let history = h.state().session.history_len().0;
            click(&mut h, "text.animation.fade_in_out");
            assert_eq!(h.state().session.history_len().0, history + 1);
            let fade = item(&h, f.text).animations;
            assert_eq!(
                fade.get(&project::BuiltinPath::Opacity.into())
                    .unwrap()
                    .channel(None)
                    .unwrap()
                    .keys()
                    .len(),
                4
            );
            h.state_mut().activate("edit.undo").unwrap();
            h.state_mut()
                .activate("text.animation.fade_in_out")
                .unwrap();
            h.state_mut()
                .dispatch(vibecut::AppCommand::Seek(Tick(38_102_400_000)))
                .unwrap();
            h.run_steps(3);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !h.state().playback.frame().is_some_and(|frame| {
                frame.generation == h.state().playback.generation()
                    && frame.at == Tick(33_868_800_000)
            }) {
                h.run_steps(1);
                assert!(std::time::Instant::now() < deadline, "title preview frame");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
            std::fs::create_dir_all(&root).unwrap();
            let image = h.render().unwrap();
            image
                .save(root.join(format!("p1-58-text-{locale:?}-{scale}x.png")))
                .unwrap();
            if locale == vibecut::i18n::Locale::En && scale == 1.0 {
                image.save(root.join("p1-58-text-extras.png")).unwrap();
            }
            click(&mut h, "inspector.tab.blending");
            assert_eq!(
                h.get(By::new().predicate(|n| n.author_id() == Some("inspector.opacity")))
                    .value()
                    .as_deref(),
                Some("30%")
            );
            click(&mut h, "inspector.opacity.increase");
            let at = Tick(38_102_400_000);
            let value = |h: &Harness<Editor>| {
                item(h, f.text)
                    .animations
                    .value_at(&project::BuiltinPath::Opacity.into(), at)
                    .unwrap()
            };
            assert_eq!(value(&h), Some(project::ParamValue::Scalar(0.31)));
            let history = h.state().session.history_len().0;
            for (finish, text, expected, steps) in [
                (egui::Key::Enter, "45", 0.45, 1),
                (egui::Key::Escape, "80", 0.45, 1),
            ] {
                click(&mut h, "inspector.opacity");
                h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
                h.event(egui::Event::Text(text.into()));
                h.key_press(finish);
                h.run_steps(3);
                assert_eq!(value(&h), Some(project::ParamValue::Scalar(expected)));
                assert_eq!(h.state().session.history_len().0, history + steps);
            }
            drag(&mut h, "inspector.opacity.scrub", egui::vec2(20.0, 0.0));
            assert_eq!(value(&h), Some(project::ParamValue::Scalar(0.5)));
            let history = h.state().session.history_len();
            drag_to(&mut h, "inspector.opacity.scrub", egui::vec2(20.0, 0.0));
            h.key_press(egui::Key::Escape);
            h.run_steps(2);
            h.drop_at(egui::pos2(700.0, 300.0));
            h.run_steps(2);
            assert_eq!(value(&h), Some(project::ParamValue::Scalar(0.5)));
            assert_eq!(h.state().session.history_len(), history);
            click(&mut h, "inspector.opacity");
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.event(egui::Event::Text("51".into()));
            h.key_press(egui::Key::Tab);
            h.run_steps(3);
            assert_eq!(value(&h), Some(project::ParamValue::Scalar(0.51)));
            assert_eq!(h.state().session.history_len().0, history.0 + 1);
            let file = root.join(format!("p1-388-{locale:?}-{scale}x.vibecut"));
            project::WriterLease::acquire(&file)
                .unwrap()
                .save(h.state().session.project())
                .unwrap();
            let reopened = project::load(&file).unwrap().project;
            assert_eq!(reopened, **h.state().session.project());
            h.state_mut().session = Memory(Box::new(Session::new(reopened, 10).unwrap()));
            h.run_steps(3);
            h.state_mut()
                .dispatch(vibecut::AppCommand::Seek(at))
                .unwrap();
            h.run_steps(3);
            click(&mut h, "inspector.opacity.increase");
            assert_eq!(value(&h), Some(project::ParamValue::Scalar(0.52)));
            let times: Vec<_> = item(&h, f.text)
                .animations
                .get(&project::BuiltinPath::Opacity.into())
                .unwrap()
                .channel(None)
                .unwrap()
                .keys()
                .iter()
                .map(|k| k.time)
                .collect();
            for time in times {
                h.state_mut()
                    .session
                    .apply(Command::RemoveKeyframe {
                        item: f.text,
                        path: project::BuiltinPath::Opacity.into(),
                        component: None,
                        time,
                    })
                    .unwrap();
            }
            h.run_steps(3);
            assert_eq!(value(&h), Some(project::ParamValue::Scalar(0.6)));
            assert_eq!(
                h.get(By::new().predicate(|n| n.author_id() == Some("inspector.opacity")))
                    .value()
                    .as_deref(),
                Some("60%")
            );
        }
    }
}

#[test]
fn opacity_drafts_invalidate_before_blur_and_keyed_outside_edits_refuse() {
    for change in [
        "time", "target", "lock", "tab", "snapshot", "undo", "remove", "scene", "project",
    ] {
        let mut f = fixture();
        f.editor.selection = [f.text].into_iter().collect();
        f.editor.activate("text.animation.fade_in_out").unwrap();
        let mut h = build(f.editor);
        h.run_steps(3);
        click(&mut h, "inspector.tab.blending");
        click(&mut h, "inspector.opacity");
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("82".into()));
        h.run_steps(2);
        let track = h
            .state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .find(|t| t.items.iter().any(|i| i.id == f.text))
            .unwrap()
            .id;
        match change {
            "time" => h
                .state_mut()
                .dispatch(vibecut::AppCommand::Seek(Tick(SECOND)))
                .unwrap(),
            "target" => h.state_mut().selection = [f.video].into_iter().collect(),
            "lock" => {
                h.state_mut()
                    .session
                    .apply(Command::SetTrackLocked {
                        track,
                        locked: true,
                    })
                    .unwrap();
            }
            "tab" => {
                h.state_mut()
                    .preferences
                    .properties_tabs
                    .insert("text".into(), "text".into());
            }
            "snapshot" => {
                h.state_mut()
                    .session
                    .apply(Command::RenameProject {
                        name: "changed".into(),
                    })
                    .unwrap();
            }
            "undo" => {
                h.state_mut().activate("edit.undo").unwrap();
            }
            "remove" => {
                h.state_mut()
                    .session
                    .apply(Command::Delete {
                        items: vec![f.text],
                    })
                    .unwrap();
            }
            "scene" => {
                let out = h
                    .state_mut()
                    .session
                    .apply(Command::CreateScene {
                        name: "other".into(),
                    })
                    .unwrap();
                let CreatedId::Scene(scene) = out.created[0] else {
                    panic!("scene")
                };
                h.state_mut()
                    .session
                    .apply(Command::SwitchScene { scene })
                    .unwrap();
            }
            "project" => {
                let empty = Project::empty(ProjectId(388), "other".into()).unwrap();
                h.state_mut().session = Memory(Box::new(Session::new(empty, 10).unwrap()));
            }
            _ => unreachable!(),
        }
        let before = h.state().session.project().clone();
        let history = h.state().session.history_len();
        h.run_steps(3);
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert_eq!(h.state().session.project(), &before, "{change}");
        assert_eq!(h.state().session.history_len(), history, "{change}");
    }
    let mut f = fixture();
    f.editor.selection = [f.text].into_iter().collect();
    f.editor.activate("text.animation.fade_in_out").unwrap();
    let track = f
        .editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|t| t.items.iter().any(|i| i.id == f.text))
        .unwrap()
        .id;
    f.editor
        .session
        .execute(
            "move_item",
            serde_json::json!({"item":f.text,"track":track,"start":SECOND}),
        )
        .unwrap();
    for (at, value) in [
        (Tick::ZERO, "0.8"),
        (Tick(3 * SECOND), "0.8"),
        (Tick(SECOND), "2"),
        (Tick(SECOND), "NaN"),
    ] {
        f.editor.dispatch(vibecut::AppCommand::Seek(at)).unwrap();
        let before = f.editor.session.project().clone();
        assert!(f.editor.set_item_field(f.text, "opacity", value).is_err());
        assert_eq!(f.editor.session.project(), &before);
    }
}

#[test]
fn text_fade_preflight_preserves_focused_and_unfocused_caption_and_properties_drafts() {
    let mut f = fixture();
    let cues = captions::parse(
        b"1\n00:00:00,000 --> 00:00:02,000\nCaption\n",
        captions::Format::Srt,
        captions::LanguageTag::parse("en").unwrap(),
    )
    .unwrap()
    .track;
    let cue = cues.cues[0].id.clone();
    let track = add_track(&mut f.editor.session, "Captions", TrackKind::Text);
    insert(
        &mut f.editor.session,
        track,
        Tick::ZERO,
        Tick(2 * SECOND),
        Some(AssetId(1)),
        ItemParams::Caption(project::CaptionParams { cues }),
    );
    f.editor.preferences.assets_tab = AssetsTab::Captions;
    f.editor.prepare_subtitles().unwrap();
    let mut h = build(f.editor);
    h.run_steps(3);
    h.get(
        By::new().predicate(|n| {
            n.author_id() == Some(format!("subtitles.cue.{}", cue.as_str()).as_str())
        }),
    )
    .click_accesskit();
    h.run_steps(3);
    select(&mut h, f.text);
    let field = format!("subtitles.text.{}.en", cue.as_str());
    h.get(By::new().predicate(|n| n.author_id() == Some(field.as_str())))
        .focus();
    h.run_steps(2);
    for dirty in [false, true] {
        if dirty {
            h.get(By::new().predicate(|n| n.author_id() == Some(field.as_str())))
                .type_text(" pending");
            h.run_steps(2);
        }
        let before = h.state().session.project().clone();
        let selection = h.state().selection.clone();
        let history = h.state().session.history_len();
        assert!(!h.state().action_enabled("text.animation.fade_in"));
        assert!(h.state_mut().activate("text.animation.fade_in").is_err());
        assert_eq!(h.state().session.project(), &before);
        assert_eq!(h.state().session.history_len(), history);
        assert_eq!(h.state().selection, selection);
        assert!(
            h.get(By::new().predicate(|n| n.author_id() == Some("text.animation.fade_in")))
                .accesskit_node()
                .is_disabled()
        );
        assert!(h.state_mut().activate("subtitles.style.box").is_err());
    }
    // Switch away without activating/flushing: the changed caption draft is now unfocused.
    h.ctx
        .memory_mut(|m| m.surrender_focus(m.focused().unwrap()));
    h.state_mut().preferences.assets_tab = AssetsTab::Media;
    h.run_steps(3);
    let before = h.state().session.project().clone();
    assert!(h.state_mut().activate("text.animation.fade_in").is_err());
    assert_eq!(h.state().session.project(), &before);
    h.state_mut().subtitles = None;
    h.run_steps(2);
    click(&mut h, "inspector.text_size");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("83".into()));
    h.run_steps(2);
    let before = h.state().session.project().clone();
    assert!(h.state_mut().activate("text.animation.fade_in").is_err());
    assert_eq!(h.state().session.project(), &before);
    // The inspector caller must see its actual held State, so even an AccessKit click cannot apply.
    h.get(By::new().predicate(|n| n.author_id() == Some("text.animation.fade_in")))
        .click_accesskit();
    h.run_steps(2);
    assert_eq!(h.state().session.project(), &before);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    for selected in [vec![f.video], vec![f.text, f.video]] {
        h.state_mut().selection = selected.into_iter().collect();
        h.run_steps(2);
        assert!(!has_widget(&h, "text.animation.fade_in"));
        assert!(h.state_mut().activate("text.animation.fade_in").is_err());
    }
    select(&mut h, f.text);
    h.state_mut().session.begin_gesture().unwrap();
    assert!(h.state_mut().activate("text.animation.fade_in").is_err());
    h.state_mut().session.cancel_gesture().unwrap();
    let track = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|t| t.items.iter().any(|i| i.id == f.text))
        .unwrap()
        .id;
    h.state_mut()
        .session
        .execute(
            "track.set_locked",
            serde_json::json!({"track":track.0,"locked":true}),
        )
        .unwrap();
    h.run_steps(2);
    assert!(
        h.get(By::new().predicate(|n| n.author_id() == Some("text.animation.fade_in")))
            .accesskit_node()
            .is_disabled()
    );
    assert!(h.state_mut().activate("text.animation.fade_in").is_err());
    h.state_mut()
        .session
        .execute(
            "track.set_locked",
            serde_json::json!({"track":track.0,"locked":false}),
        )
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let session = Session::new((**h.state().session.project()).clone(), 10).unwrap();
    let mut saving = engine::recovery::Autosave::new(
        &root.path().join("recovery"),
        session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(engine::jobs::SystemClock::default()),
        Default::default(),
    )
    .unwrap();
    saving.save_as(root.path().join("title.vibecut")).unwrap();
    h.state_mut().session = vibecut::lifecycle::EditorSession::Persistent(Box::new(saving));
    let before = h.state().session.project().clone();
    assert!(h.state_mut().activate("text.animation.fade_in").is_err());
    assert_eq!(h.state().session.project(), &before);
    h.state_mut().session.autosave().unwrap().flush().unwrap();
    drop(h);

    let mut f = fixture();
    f.editor
        .session
        .apply(Command::Delete {
            items: vec![f.video, f.silent_video, f.image, f.graphic, f.audio],
        })
        .unwrap();
    f.editor.set_item_field(f.text, "text_size", "49").unwrap();
    f.editor.session.undo().unwrap();
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_step_dt(0.05)
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), f.editor);
    select(&mut h, f.text);
    let point = widget_rect(&h, "preview.transform.left")
        .center()
        .lerp(widget_rect(&h, "preview.transform.right").center(), 0.5);
    for _ in 0..2 {
        h.event(egui::Event::PointerMoved(point));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
    }
    h.run_steps(3);
    assert!(
        has_widget(&h, "preview.text.edit"),
        "real clean inline editor opens"
    );
    assert!(!h.state().session.gesture_active());
    assert_eq!(h.state().session.history_len().1, 1);
    for dirty in [false, true] {
        if dirty {
            h.get(By::new().predicate(|n| n.author_id() == Some("preview.text.edit")))
                .type_text(" pending");
            h.run_steps(2);
            assert!(h.state().session.gesture_active());
        }
        let before = h.state().session.project().clone();
        let candidate = h.state().session.preview_project().cloned();
        let history = h.state().session.history_len();
        assert!(h.state_mut().activate("text.animation.fade_in").is_err());
        assert!(!h.state().action_enabled("text.animation.fade_in"));
        assert_eq!(h.state().session.project(), &before);
        assert_eq!(h.state().session.preview_project(), candidate.as_ref());
        assert_eq!(h.state().session.history_len(), history);
        assert!(
            has_widget(&h, "preview.text.edit"),
            "refusal leaves editor open"
        );
        assert!(
            h.get(By::new().predicate(|n| n.author_id() == Some("text.animation.fade_in")))
                .accesskit_node()
                .is_disabled()
        );
    }
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!has_widget(&h, "preview.text.edit"));
    assert!(h.state().action_enabled("text.animation.fade_in"));
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len();
    h.state_mut().activate("text.animation.fade_in").unwrap();
    assert_eq!(h.state().session.history_len(), (history.0 + 1, 0));
    h.state_mut().activate("edit.undo").unwrap();
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn speed_tab_scrubs_resets_and_toggles_pitch_through_one_command() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.video);
    click(&mut harness, "inspector.tab.speed");
    assert!(has_widget(&harness, "inspector.speed"));
    assert!(has_widget(&harness, "inspector.speed.duration"));
    assert_eq!(
        item(&harness, fixture.video).retime,
        project::Retime::default()
    );

    // 100 px at 2 px per 0.01 step is +0.5x, one undo step; the clip shortens to match.
    let history = harness.state().session.history_len();
    drag(
        &mut harness,
        "inspector.speed.scrub",
        egui::vec2(100.0, 0.0),
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    let scrubbed = item(&harness, fixture.video);
    assert_eq!(scrubbed.retime.speed.as_f64(), 1.5);
    assert_eq!(
        scrubbed.duration().unwrap(),
        Tick(2 * SECOND * 2 / 3),
        "duration follows speed over the same source range"
    );

    // "Change pitch" starts off (pitch preserved); turning it on lets pitch follow speed.
    assert!(item(&harness, fixture.video).retime.maintain_pitch);
    click(&mut harness, "inspector.speed.pitch");
    let kept = item(&harness, fixture.video);
    assert!(!kept.retime.maintain_pitch);
    assert_eq!(kept.retime.speed.as_f64(), 1.5);

    // Reset returns to 1x, keeps the pitch policy, and is inert afterwards.
    click(&mut harness, "inspector.speed.reset");
    let reset = item(&harness, fixture.video);
    assert_eq!(reset.retime.speed, time::Speed::UNITY);
    assert!(!reset.retime.maintain_pitch, "reset keeps the pitch policy");
    assert_eq!(reset.duration().unwrap(), Tick(2 * SECOND));
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.speed.reset");
    assert_eq!(harness.state().session.history_len(), history);

    // Three committed edits undo one by one.
    for _ in 0..3 {
        harness.state_mut().session.undo().unwrap();
    }
    assert_eq!(
        item(&harness, fixture.video).retime,
        project::Retime::default()
    );

    // Audio items get the same controls; a video without audio has nothing to pitch-shift.
    select(&mut harness, fixture.audio);
    click(&mut harness, "inspector.tab.speed");
    assert!(has_widget(&harness, "inspector.speed.pitch"));
    select(&mut harness, fixture.silent_video);
    click(&mut harness, "inspector.tab.speed");
    assert!(has_widget(&harness, "inspector.speed"));
    assert!(!has_widget(&harness, "inspector.speed.pitch"));
}

#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-41-retime.png"]
fn speed_tab_renders_the_retimed_clip_controls() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.video);
    click(&mut harness, "inspector.tab.speed");
    drag(
        &mut harness,
        "inspector.speed.scrub",
        egui::vec2(100.0, 0.0),
    );
    click(&mut harness, "inspector.speed.pitch");
    harness.run_steps(3);
    let image = harness.render().expect("offscreen render");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-41-retime.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
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
    click(&mut harness, "inspector.tab.adjust");
    store.save(&harness.state().preferences).unwrap();
    let restored = store.load().unwrap();
    assert_eq!(
        restored.properties_tabs.get("image").map(String::as_str),
        Some("adjust")
    );
    let restarted_fixture = fixture();
    let restarted_image = restarted_fixture.image;
    let mut restarted = build(restarted_fixture.editor);
    restarted.state_mut().preferences = restored;
    restarted.run_steps(3);
    select(&mut restarted, restarted_image);
    assert!(has_widget(&restarted, "inspector.tab.adjust"));
    assert_eq!(
        restarted
            .state()
            .preferences
            .properties_tabs
            .get("image")
            .map(String::as_str),
        Some("adjust")
    );
}

fn text_size(harness: &Harness<Editor>, id: ItemId) -> f32 {
    let ItemParams::Text(params) = &item(harness, id).content else {
        panic!("text")
    };
    params.font_size
}

fn text_params(harness: &Harness<Editor>, id: ItemId) -> project::TextParams {
    let ItemParams::Text(params) = item(harness, id).content else {
        panic!("text")
    };
    params
}

#[test]
fn text_tab_edits_every_style_field_one_undo_step_each_and_shows_effects() {
    use project::{TextAlign, TextDecoration};
    let mut fixture = fixture();
    // A healthy generated preview for the screenshot, as the colour test does.
    fixture
        .editor
        .session
        .apply(Command::Delete {
            items: vec![
                fixture.video,
                fixture.silent_video,
                fixture.image,
                fixture.audio,
            ],
        })
        .unwrap();
    fixture
        .editor
        .playback
        .seek(Tick::ZERO, false, fixture.editor.session.project().clone())
        .unwrap();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, fixture.text);
    let text = fixture.text;
    // Each toggle row is one registered edit, one undo step, and reads back what it wrote.
    type Check = fn(&project::TextParams) -> bool;
    let toggles: [(&str, Check); 7] = [
        ("inspector.text_weight.bold", |p| p.weight == 700),
        ("inspector.text_weight.normal", |p| p.weight == 400),
        ("inspector.text_italic.italic", |p| p.italic),
        ("inspector.text_decoration.underline", |p| {
            p.decoration == TextDecoration::Underline
        }),
        ("inspector.text_decoration.line_through", |p| {
            p.decoration == TextDecoration::LineThrough
        }),
        ("inspector.text_align.left", |p| p.align == TextAlign::Left),
        ("inspector.text_align.right", |p| {
            p.align == TextAlign::Right
        }),
    ];
    for (id, holds) in toggles {
        let before = harness.state().session.project().clone();
        click(&mut harness, id);
        assert!(holds(&text_params(&harness, text)), "{id}");
        harness.state_mut().session.undo().unwrap();
        assert_eq!(harness.state().session.project(), &before, "{id} undo");
        harness.state_mut().session.redo().unwrap();
        harness.run_steps(2);
    }
    // Turn the box on with the shared toggle, then every numeric row commits through the field.
    click(&mut harness, "inspector.text_background");
    assert!(text_params(&harness, text).background.is_some());
    for (id, value, holds) in [
        (
            "inspector.text_letter_spacing",
            "3.5",
            (|p: &project::TextParams| p.letter_spacing == 3.5) as Check,
        ),
        ("inspector.text_line_height", "44", |p| {
            p.line_height == 44.0
        }),
        ("inspector.text_background_padding_x", "20", |p| {
            p.background.as_ref().unwrap().padding_x == 20.0
        }),
        ("inspector.text_background_padding_y", "8", |p| {
            p.background.as_ref().unwrap().padding_y == 8.0
        }),
        ("inspector.text_background_offset_x", "-14", |p| {
            p.background.as_ref().unwrap().offset_x == -14.0
        }),
        ("inspector.text_background_offset_y", "6", |p| {
            p.background.as_ref().unwrap().offset_y == 6.0
        }),
        ("inspector.text_background_corner_radius", "12", |p| {
            p.background.as_ref().unwrap().radius == 12.0
        }),
    ] {
        let before = harness.state().session.project().clone();
        enter(&mut harness, id, value);
        assert!(holds(&text_params(&harness, text)), "{id}");
        harness.state_mut().session.undo().unwrap();
        assert_eq!(harness.state().session.project(), &before, "{id} undo");
        harness.state_mut().session.redo().unwrap();
        harness.run_steps(2);
    }
    // Hostile entries clamp to the field's range instead of reaching the model.
    enter(&mut harness, "inspector.text_background_padding_x", "9999");
    assert_eq!(
        text_params(&harness, text).background.unwrap().padding_x,
        project::TextBackground::MAX_EXTENT
    );
    enter(&mut harness, "inspector.text_background_offset_y", "-99999");
    assert_eq!(
        text_params(&harness, text).background.unwrap().offset_y,
        -project::TextBackground::MAX_OFFSET
    );
    let before = harness.state().session.project().clone();
    enter(&mut harness, "inspector.text_letter_spacing", "wide");
    assert_eq!(
        harness.state().session.project(),
        &before,
        "words are refused"
    );
    enter(&mut harness, "inspector.text_background_padding_x", "20");

    // Text exposes the shared Effects tab beside Transform and Blending.
    assert!(has_widget(&harness, "inspector.tab.effects"));
    assert!(has_widget(&harness, "inspector.group.basic"));
    let image = harness.render().expect("offscreen render");
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-27-text.png")).unwrap();
    click(&mut harness, "inspector.tab.effects");
    assert!(has_widget(&harness, "effects.stack.empty"));
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

#[derive(Clone)]
struct FakeFonts {
    inner: Arc<Mutex<FakeInner>>,
}
struct FakeInner {
    commands: Vec<&'static str>,
    fail_list: bool,
    entries: Vec<FontEntry>,
    loads: BTreeMap<String, FontLoadState>,
    job_state: JobState,
    job_progress: f32,
    fail_download: bool,
}
impl FakeFonts {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeInner {
                commands: Vec::new(),
                fail_list: false,
                entries: vec![
                    font_entry("Inter", FontOrigin::Bundled, FontState::Available, None),
                    font_entry("Broken", FontOrigin::Bundled, FontState::Available, None),
                    font_entry(
                        "Noto Serif",
                        FontOrigin::Bundled,
                        FontState::Available,
                        None,
                    ),
                    font_entry(
                        "Lobster",
                        FontOrigin::Google,
                        FontState::NotDownloaded,
                        Some(61_000),
                    ),
                    font_entry("Arial", FontOrigin::System, FontState::Available, None),
                ],
                loads: BTreeMap::from([("Broken".into(), FontLoadState::NotDownloaded)]),
                job_state: JobState::Running,
                job_progress: 0.4,
                fail_download: false,
            })),
        }
    }
    fn commands(&self) -> Vec<&'static str> {
        self.inner.lock().unwrap().commands.clone()
    }
    fn count(&self, command: &str) -> usize {
        self.commands().iter().filter(|id| **id == command).count()
    }
    fn fail_list(&self, fail: bool) {
        self.inner.lock().unwrap().fail_list = fail;
    }
    fn fail_download(&self, fail: bool) {
        self.inner.lock().unwrap().fail_download = fail;
    }
    fn set_job(&self, state: JobState, progress: f32) {
        let mut inner = self.inner.lock().unwrap();
        inner.job_state = state;
        inner.job_progress = progress;
    }
    fn mark_available(&self, family: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(entry) = inner
            .entries
            .iter_mut()
            .find(|entry| entry.family == family)
        {
            entry.state = FontState::Available;
            entry.download_bytes = None;
        }
    }
}
impl Backend for FakeFonts {
    fn list(&mut self, _: &mut Session, _: Option<&str>) -> vibecut::Result<Vec<FontEntry>> {
        let mut inner = self.inner.lock().unwrap();
        inner.commands.push("fonts.list");
        if inner.fail_list {
            return Err(vibecut::Error::Invalid("font list"));
        }
        Ok(inner.entries.clone())
    }
    fn load(&mut self, _: &mut Session, family: &str) -> vibecut::Result<FontLoad> {
        let mut inner = self.inner.lock().unwrap();
        inner.commands.push("fonts.load");
        let state = inner
            .loads
            .get(family)
            .copied()
            .unwrap_or(FontLoadState::Loaded);
        Ok(FontLoad {
            family: family.to_owned(),
            state,
            origin: Some(FontOrigin::Bundled),
            effective_family: if state == FontLoadState::Loaded {
                family.to_owned()
            } else {
                "Inter".into()
            },
            styles: vec!["Regular".into()],
        })
    }
    fn download(&mut self, _: &mut Session, family: &str) -> vibecut::Result<JobId> {
        let mut inner = self.inner.lock().unwrap();
        inner.commands.push("fonts.download");
        if inner.fail_download {
            return Err(vibecut::Error::Invalid("font download"));
        }
        inner.job_state = JobState::Running;
        inner.job_progress = 0.0;
        let _ = family;
        Ok(JobId(7))
    }
    fn job(&mut self, job: JobId) -> Option<JobView> {
        let inner = self.inner.lock().unwrap();
        Some(JobView {
            id: job,
            kind: JobKind::DownloadFont,
            stage: JobStage::Download,
            progress: inner.job_progress,
            state: inner.job_state,
            attempts: 0,
            error: None,
            output: None,
        })
    }
    fn cancel(&mut self, _: &mut Session, _: JobId) -> vibecut::Result<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.commands.push("job_cancel");
        inner.job_state = JobState::Cancelling;
        Ok(())
    }
}
fn font_entry(
    family: &str,
    origin: FontOrigin,
    state: FontState,
    download_bytes: Option<u64>,
) -> FontEntry {
    FontEntry {
        family: family.into(),
        origin,
        styles: vec!["Regular".into()],
        state,
        license: (origin == FontOrigin::Google).then(|| "OFL-1.1".into()),
        source: (origin == FontOrigin::Google).then(|| "https://github.com/google/fonts".into()),
        download_bytes,
    }
}
fn fonts_editor(fixture: Fixture, fonts: &FakeFonts) -> (Editor, ItemId) {
    let mut editor = fixture.editor;
    editor.set_font_backend(Box::new(fonts.clone()));
    (editor, fixture.text)
}
fn open_font_picker(harness: &mut Harness<Editor>) {
    harness.run_steps(2);
    click(harness, "inspector.text_font");
    // Frame one draws Loading, then the list request is applied.
    harness.run_steps(1);
    assert!(font_picker(harness).open, "the trigger opens the picker");
    harness.run_steps(1);
}
fn font_picker(harness: &Harness<Editor>) -> vibecut::font_picker::Inspection {
    harness.state().inspect().unwrap().font_picker
}
fn type_search(harness: &mut Harness<Editor>, text: &str) {
    click(harness, "fonts.search");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.key_press(egui::Key::Backspace);
    harness.event(egui::Event::Text(text.into()));
    harness.run_steps(2);
}

#[test]
fn font_picker_lists_search_tabs_and_favorites_through_commands() {
    let fixture = fixture();
    let fonts = FakeFonts::new();
    let (editor, text) = fonts_editor(fixture, &fonts);
    let mut harness = build(editor);
    harness.run_steps(3);
    select(&mut harness, text);
    let project = harness.state().session.project().clone();

    harness
        .get(By::new().predicate(|node| node.author_id() == Some("inspector.text_font")))
        .click();
    harness.run_steps(1);
    assert!(
        has_widget(&harness, "fonts.loading"),
        "the opening frame renders the loading state"
    );
    harness.run_steps(1);
    // The installed preview faces become usable on the following pass.
    harness.run_steps(1);
    let inspection = font_picker(&harness);
    assert_eq!(inspection.status, Status::Ready);
    assert_eq!(fonts.count("fonts.list"), 1);
    assert!(has_widget(&harness, "inspector.text_font.Inter"));
    assert!(has_widget(&harness, "fonts.download.Lobster"));
    assert!(
        inspection
            .preview_fonts
            .iter()
            .any(|family| family == "Inter"),
        "bundled rows install their preview typeface"
    );
    assert!(
        inspection
            .preview_fonts
            .iter()
            .any(|family| family == "Noto Serif"),
        "{inspection:?}"
    );
    assert_eq!(
        harness.state().session.project(),
        &project,
        "listing fonts never mutates the project"
    );

    // Every row shares the star, name and state columns, available or not.
    let field = widget_rect(&harness, "inspector.text_font");
    let search = widget_rect(&harness, "fonts.search");
    assert!(
        search.top() >= field.bottom(),
        "the popover opens below the field"
    );
    assert!(
        search.left() >= harness.state().panels()["inspector"].left(),
        "the popover stays inside the properties panel"
    );
    let star = widget_rect(&harness, "fonts.favorite.Inter");
    let star_google = widget_rect(&harness, "fonts.favorite.Lobster");
    assert_eq!(
        (star.left(), star.width()),
        (star_google.left(), star_google.width())
    );
    // Names start at the name column's left edge; the text rect hugs the label.
    let name = widget_rect(&harness, "fonts.name.Inter");
    let name_google = widget_rect(&harness, "fonts.name.Lobster");
    assert_eq!(
        name.left(),
        name_google.left(),
        "every name starts at the same left edge"
    );
    assert_eq!(
        name.left(),
        star.right(),
        "the name text starts at the name column's left edge"
    );
    // Only informative rows carry state text: not downloaded with size, and
    // later downloading or failed. Ready rows are blank.
    assert!(
        !has_widget(&harness, "fonts.state.Inter"),
        "an available row drops the Available label"
    );
    assert!(has_widget(&harness, "fonts.state.Lobster"));
    let state = widget_rect(&harness, "fonts.state.Lobster");
    let size = widget_rect(&harness, "fonts.state_size.Lobster");
    assert_eq!(
        state.left(),
        size.left(),
        "the state column has one fixed left edge"
    );
    assert!(
        size.top() >= state.bottom() && size.width() > 0.0,
        "the not-downloaded row stacks its size under its state text"
    );
    // The clickable row cell still spans the whole name column.
    let row = widget_rect(&harness, "inspector.text_font.Inter");
    assert_eq!(row.left(), name.left());
    assert!(row.right() <= state.left() + 0.5);
    assert!(widget_rect(&harness, "fonts.download.Lobster").left() >= state.left());
    // The state block is centred on the same line as the name.
    assert!(
        ((state.top() + size.bottom()) * 0.5 - name_google.center().y).abs() < 1.0,
        "the state text shares the row's centre line with the name"
    );

    type_search(&mut harness, "not");
    assert!(has_widget(&harness, "inspector.text_font.Noto Serif"));
    assert!(!has_widget(&harness, "inspector.text_font.Inter"));
    type_search(&mut harness, "zzzz");
    assert!(has_widget(&harness, "fonts.empty"));
    type_search(&mut harness, "");

    // The source tab set always shows all three; My fonts is empty until 80.
    assert!(has_widget(&harness, "fonts.tab.all"));
    assert!(has_widget(&harness, "fonts.tab.my_fonts"));
    assert!(has_widget(&harness, "fonts.tab.favorites"));
    assert!(
        harness
            .get(By::new().predicate(|node| node.author_id() == Some("fonts.upload")))
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .get(By::new().predicate(|node| node.author_id() == Some("fonts.category.custom")))
            .accesskit_node()
            .is_disabled()
    );
    click(&mut harness, "fonts.tab.my_fonts");
    assert!(
        has_widget(&harness, "fonts.my_empty"),
        "no imports exist yet"
    );
    click(&mut harness, "fonts.tab.all");

    click(&mut harness, "fonts.tab.favorites");
    assert!(has_widget(&harness, "fonts.empty"));
    click(&mut harness, "fonts.tab.all");
    click(&mut harness, "fonts.favorite.Noto Serif");
    assert_eq!(
        harness.state().preferences.font_picker.favorites,
        ["Noto Serif"]
    );
    assert_eq!(
        harness.state().session.project(),
        &project,
        "favoriting never mutates the project"
    );
    click(&mut harness, "fonts.tab.favorites");
    assert!(has_widget(&harness, "inspector.text_font.Noto Serif"));
    assert!(!has_widget(&harness, "inspector.text_font.Inter"));

    let history = harness.state().session.history_len().0;
    click(&mut harness, "inspector.text_font.Noto Serif");
    harness.run_steps(2);
    assert_eq!(fonts.count("fonts.load"), 1);
    assert_eq!(harness.state().session.history_len().0, history + 1);
    let ItemParams::Text(text) = &item(&harness, text).content else {
        panic!("text item");
    };
    assert_eq!(text.font.as_str(), "Noto Serif");
    assert!(!font_picker(&harness).open);

    // The persisted tab and favourite survive close and reopen.
    open_font_picker(&mut harness);
    let inspection = font_picker(&harness);
    assert!(inspection.open, "{inspection:?}");
    assert_eq!(inspection.tab, FontTab::Favorites, "{inspection:?}");
    assert_eq!(inspection.status, Status::Ready, "{inspection:?}");
    assert!(inspection.favorites.iter().any(|name| name == "Noto Serif"));
    assert!(
        has_widget(&harness, "inspector.text_font.Noto Serif"),
        "{inspection:?}"
    );
}

#[test]
fn font_picker_error_retry_and_failed_preview_fallback() {
    let fixture = fixture();
    let fonts = FakeFonts::new();
    fonts.fail_list(true);
    let (editor, text) = fonts_editor(fixture, &fonts);
    let mut harness = build(editor);
    harness.run_steps(3);
    select(&mut harness, text);
    let project = harness.state().session.project().clone();
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("inspector.text_font")))
        .click();
    harness.run_steps(3);
    assert_eq!(font_picker(&harness).status, Status::Failed);
    assert!(has_widget(&harness, "fonts.error"));
    assert!(has_widget(&harness, "fonts.retry"));
    assert_eq!(
        harness.state().session.project(),
        &project,
        "a failed list leaves the project untouched"
    );
    fonts.fail_list(false);
    click(&mut harness, "fonts.retry");
    harness.run_steps(2);
    assert_eq!(font_picker(&harness).status, Status::Ready);
    assert_eq!(fonts.count("fonts.list"), 2);

    // A failed registration falls back silently to the next available family.
    let history = harness.state().session.history_len().0;
    click(&mut harness, "inspector.text_font.Broken");
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history + 1);
    let ItemParams::Text(text) = &item(&harness, text).content else {
        panic!("text item");
    };
    assert_eq!(text.font.as_str(), "Noto Serif");
    assert_eq!(font_picker(&harness).status, Status::Ready);
    assert!(!font_picker(&harness).open);
}

#[test]
fn font_picker_download_cancel_and_failure_states() {
    let fixture = fixture();
    let fonts = FakeFonts::new();
    let (editor, text) = fonts_editor(fixture, &fonts);
    let mut harness = build(editor);
    harness.run_steps(3);
    select(&mut harness, text);
    open_font_picker(&mut harness);
    let project = harness.state().session.project().clone();

    click(&mut harness, "fonts.download.Lobster");
    harness.run_steps(2);
    let download = font_picker(&harness).download.expect("download started");
    assert_eq!(download.family, "Lobster");
    assert_eq!(fonts.count("fonts.download"), 1);
    assert!(has_widget(&harness, "fonts.downloading"));
    assert!(has_widget(&harness, "fonts.cancel"));
    assert_eq!(
        harness.state().session.project(),
        &project,
        "downloading never mutates the project"
    );

    click(&mut harness, "fonts.cancel");
    harness.run_steps(2);
    assert_eq!(fonts.count("job_cancel"), 1);
    assert!(font_picker(&harness).download.unwrap().cancelling);
    assert!(has_widget(&harness, "fonts.downloading"));
    fonts.set_job(JobState::Cancelled, 0.4);
    harness.run_steps(2);
    assert!(font_picker(&harness).download.is_none());

    // The submission itself can be refused without a job.
    fonts.fail_download(true);
    click(&mut harness, "fonts.download.Lobster");
    harness.run_steps(2);
    assert!(font_picker(&harness).download.unwrap().failed);
    assert!(has_widget(&harness, "fonts.download_failed"));
    fonts.fail_download(false);

    // A job that fails keeps its family for an explicit retry.
    click(&mut harness, "fonts.download_retry.Lobster");
    harness.run_steps(1);
    fonts.set_job(JobState::Failed, 0.4);
    harness.run_steps(2);
    assert!(font_picker(&harness).download.unwrap().failed);

    fonts.mark_available("Lobster");
    click(&mut harness, "fonts.download_retry.Lobster");
    harness.run_steps(1);
    fonts.set_job(JobState::Completed, 1.0);
    harness.run_steps(3);
    assert!(font_picker(&harness).download.is_none());
    assert!(
        fonts.count("fonts.list") >= 2,
        "a finished download relists"
    );
    assert!(has_widget(&harness, "inspector.text_font.Lobster"));
}

#[test]
fn font_picker_screenshot_is_available() {
    let fixture = fixture();
    let fonts = FakeFonts::new();
    let (editor, text) = fonts_editor(fixture, &fonts);
    let mut harness = build(editor);
    harness.run_steps(3);
    select(&mut harness, text);
    open_font_picker(&mut harness);
    click(&mut harness, "fonts.favorite.Noto Serif");
    let image = harness.render().expect("offscreen render");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-28-fonts.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image.save(&path).unwrap();
    assert!(path.exists());
}
