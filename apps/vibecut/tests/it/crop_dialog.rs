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
use media::{AssetId, ContentHash, SourceAsset, SourceStream};
use project::{ItemId, ItemParams, Project, ProjectId, TrackId, TrackKind};
use std::{path::PathBuf, sync::Arc};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    CropAspect, Editor, egui,
    preferences::Preferences,
    services::playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;
const SOURCE_ASPECT: f64 = 1920.0 / 1080.0;

struct Fixture {
    editor: Editor,
    video: ItemId,
    image: ItemId,
    other_video: ItemId,
    text: ItemId,
    audio: ItemId,
}

fn stream(kind: media::StreamKind, frame_count: Option<u64>) -> SourceStream {
    let mut builder = testkit::source::stream(kind).duration(Tick(10 * SECOND));
    if let Some(frame_count) = frame_count {
        builder = builder.frame_count(frame_count);
    }
    builder.build()
}

fn asset(id: u64, streams: Vec<SourceStream>) -> SourceAsset {
    testkit::source::asset(
        id,
        std::env::temp_dir().join(format!("vibecut-crop-{id}.mp4")),
    )
    .format("mov,mp4")
    .duration(Tick(10 * SECOND))
    .content_hash(ContentHash::from_bytes(&[id as u8; 8]))
    .streams(
        streams
            .into_iter()
            .enumerate()
            .map(|(index, mut stream)| {
                stream.index = index;
                stream
            })
            .collect::<Vec<_>>(),
    )
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

fn video_params() -> ItemParams {
    ItemParams::Video(project::VideoParams::plain())
}

fn fixture() -> Fixture {
    let mut session =
        Session::new(Project::empty(ProjectId(173), "Crop".into()).unwrap(), 50).unwrap();
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
        video_params(),
    );
    let image = insert(
        &mut session,
        main,
        Tick(2 * SECOND),
        Tick(SECOND),
        Some(AssetId(3)),
        video_params(),
    );
    let other_video = insert(
        &mut session,
        main,
        Tick(3 * SECOND),
        Tick(SECOND),
        Some(AssetId(4)),
        video_params(),
    );
    let text = insert(
        &mut session,
        text_track,
        Tick::ZERO,
        Tick(2 * SECOND),
        None,
        ItemParams::Text(project::TextParams {
            content: "Hello".into(),
            ..Default::default()
        }),
    );
    let audio = insert(
        &mut session,
        audio_track,
        Tick::ZERO,
        Tick(2 * SECOND),
        Some(AssetId(2)),
        ItemParams::Audio(project::AudioParams::default()),
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
        other_video,
        text,
        audio,
    }
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn select(harness: &mut Harness<Editor>, items: &[ItemId]) {
    harness
        .state_mut()
        .select(items.iter().copied().collect())
        .unwrap();
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

fn enabled(harness: &Harness<Editor>, id: &str) -> bool {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .find(|widget| widget.id == id)
        .unwrap_or_else(|| panic!("missing widget {id}"))
        .enabled
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

/// The open dialog's working rectangle.
fn working(harness: &Harness<Editor>) -> vibecut::CropInspection {
    harness
        .state()
        .inspect()
        .unwrap()
        .crop
        .expect("crop dialog open")
}

fn item_crop(harness: &Harness<Editor>, id: ItemId) -> Option<project::Crop> {
    let item = harness
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
        .unwrap();
    match item.content {
        ItemParams::Video(params) => params.crop,
        _ => panic!("not a visual item"),
    }
}

/// An independent expectation: the largest centred box of `ratio` (output
/// pixels) inside a 1920x1080 source.
fn shape(ratio: f64) -> project::Crop {
    let normalized = ratio / SOURCE_ASPECT;
    let (width, height) = if normalized >= 1.0 {
        (1.0, 1.0 / normalized)
    } else {
        (normalized, 1.0)
    };
    project::Crop {
        x: (1.0 - width) / 2.0,
        y: (1.0 - height) / 2.0,
        width,
        height,
    }
}

fn close_to(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}

fn same_box(a: project::Crop, b: project::Crop) -> bool {
    close_to(a.x, b.x)
        && close_to(a.y, b.y)
        && close_to(a.width, b.width)
        && close_to(a.height, b.height)
}

/// The box's width/height in source pixels.
fn output_ratio(crop: project::Crop) -> f64 {
    crop.width / crop.height * SOURCE_ASPECT
}

fn assert_locked(harness: &Harness<Editor>, expected: f64, aspect: CropAspect) {
    let inspection = working(harness);
    assert_eq!(inspection.aspect, aspect);
    assert!(
        (output_ratio(inspection.crop) - expected).abs() <= 1e-6,
        "{:?}",
        inspection.crop
    );
    assert!(
        inspection.crop.valid(),
        "{:?} stays inside the frame",
        inspection.crop
    );
}

/// Screen position of a normalized point inside the preview.
fn point(harness: &Harness<Editor>, crop: project::Crop, x: f64, y: f64) -> egui::Pos2 {
    let bounds = widget_rect(harness, "crop.preview");
    egui::pos2(
        bounds.left() + bounds.width() * (crop.x + crop.width * x) as f32,
        bounds.top() + bounds.height() * (crop.y + crop.height * y) as f32,
    )
}

fn drag(harness: &mut Harness<Editor>, from: egui::Pos2, delta: egui::Vec2) {
    harness.hover_at(from);
    harness.run_steps(1);
    harness.drag_at(from);
    harness.run_steps(1);
    harness.hover_at(from + delta);
    harness.run_steps(2);
    harness.drop_at(from + delta);
    harness.run_steps(2);
}

fn open(harness: &mut Harness<Editor>, clip: ItemId) {
    select(harness, &[clip]);
    click(harness, "timeline.crop");
    assert!(has_widget(harness, "crop.apply"), "crop dialog opens");
}

fn choose(harness: &mut Harness<Editor>, option: &str) {
    click(harness, "crop.aspect");
    click(harness, option);
}

#[test]
fn crop_entry_is_available_for_one_video_or_image_only() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    assert!(!enabled(&harness, "timeline.crop"));
    assert!(harness.state_mut().activate("timeline.crop").is_err());

    for clip in [fixture.text, fixture.audio] {
        select(&mut harness, &[clip]);
        assert!(!enabled(&harness, "timeline.crop"));
        assert!(harness.state_mut().activate("timeline.crop").is_err());
    }
    for clip in [fixture.video, fixture.image] {
        select(&mut harness, &[clip]);
        assert!(enabled(&harness, "timeline.crop"));
    }
    select(&mut harness, &[fixture.video, fixture.other_video]);
    assert!(!enabled(&harness, "timeline.crop"));
}

#[test]
fn crop_ratios_drag_resize_reset_apply_and_discard() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    let entry = harness.state().session.project().clone();
    open(&mut harness, fixture.video);
    assert!(has_widget(&harness, "crop.aspect"));
    assert!(has_widget(&harness, "crop.preview"));
    assert!(has_widget(&harness, "crop.reset"));
    assert!(has_widget(&harness, "crop.close"));
    // Manual crop only: no mode, auto reframe, stabilization or speed controls.
    let mut ids: Vec<_> = harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .map(|widget| widget.id)
        .filter(|id| id.starts_with("crop."))
        .collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            "crop.apply",
            "crop.aspect",
            "crop.close",
            "crop.preview",
            "crop.reset"
        ]
    );
    assert!(Arc::ptr_eq(&entry, harness.state().session.project()));

    // Each ratio resizes the box to that shape, centred.
    for (option, expected, aspect) in [
        (
            "crop.aspect.original",
            project::Crop::default(),
            CropAspect::Original,
        ),
        (
            "crop.aspect.9x16",
            shape(9.0 / 16.0),
            CropAspect::NineSixteen,
        ),
        ("crop.aspect.1x1", shape(1.0), CropAspect::Square),
        ("crop.aspect.3x4", shape(3.0 / 4.0), CropAspect::ThreeFour),
        (
            "crop.aspect.16x9",
            shape(16.0 / 9.0),
            CropAspect::SixteenNine,
        ),
        ("crop.aspect.4x3", shape(4.0 / 3.0), CropAspect::FourThree),
        ("crop.aspect.2x1", shape(2.0), CropAspect::TwoOne),
    ] {
        choose(&mut harness, option);
        let inspection = working(&harness);
        assert_eq!(inspection.aspect, aspect, "{option}");
        assert!(
            same_box(inspection.crop, expected),
            "{option}: {:?}",
            inspection.crop
        );
        assert!(inspection.crop.valid(), "{option} stays inside the frame");
        assert!(Arc::ptr_eq(&entry, harness.state().session.project()));
    }
    // Custom keeps the current box and only frees the shape.
    let before_custom = working(&harness).crop;
    choose(&mut harness, "crop.aspect.custom");
    assert_eq!(working(&harness).aspect, CropAspect::Custom);
    assert!(same_box(working(&harness).crop, before_custom));

    // Dragging the box moves it without leaving the frame or changing shape.
    choose(&mut harness, "crop.aspect.9x16");
    let portrait = working(&harness).crop;
    let centre = point(&harness, portrait, 0.5, 0.5);
    drag(&mut harness, centre, egui::vec2(60.0, 0.0));
    let moved = working(&harness).crop;
    assert!(moved.x > portrait.x, "pointer drag moved the box");
    assert_eq!(working(&harness).aspect, CropAspect::NineSixteen);
    assert!((output_ratio(moved) - 9.0 / 16.0).abs() <= 1e-6);
    assert!(moved.valid());
    assert!(Arc::ptr_eq(&entry, harness.state().session.project()));
    assert_eq!(item_crop(&harness, fixture.video), None);

    // Reset returns the box to the full frame; applying that is a no-op.
    click(&mut harness, "crop.reset");
    assert_eq!(working(&harness).aspect, CropAspect::Original);
    assert!(same_box(working(&harness).crop, project::Crop::default()));
    click(&mut harness, "crop.apply");
    assert!(!has_widget(&harness, "crop.apply"), "apply closes");
    assert!(Arc::ptr_eq(&entry, harness.state().session.project()));
    assert_eq!(item_crop(&harness, fixture.video), None);

    // Apply writes the crop as one undo step.
    open(&mut harness, fixture.video);
    choose(&mut harness, "crop.aspect.9x16");
    let before_apply = harness.state().session.project().clone();
    click(&mut harness, "crop.apply");
    assert_eq!(item_crop(&harness, fixture.video), Some(shape(9.0 / 16.0)));
    assert!(!Arc::ptr_eq(
        &before_apply,
        harness.state().session.project()
    ));
    harness.state_mut().session.apply(Command::Undo {}).unwrap();
    assert_eq!(item_crop(&harness, fixture.video), None);
    assert!(Arc::ptr_eq(&entry, harness.state().session.project()));
    harness.state_mut().session.apply(Command::Redo {}).unwrap();
    assert_eq!(item_crop(&harness, fixture.video), Some(shape(9.0 / 16.0)));

    // Escape and the close button discard the transient box and the shape.
    let committed = harness.state().session.project().clone();
    open(&mut harness, fixture.video);
    choose(&mut harness, "crop.aspect.3x4");
    let centre = point(&harness, working(&harness).crop, 0.5, 0.5);
    drag(&mut harness, centre, egui::vec2(20.0, 0.0));
    assert!(!same_box(working(&harness).crop, shape(3.0 / 4.0)));
    let screenshot = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-81-crop.png");
    if let Some(parent) = screenshot.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    harness.render().unwrap().save(&screenshot).unwrap();
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(!has_widget(&harness, "crop.apply"), "Escape discards");
    assert!(Arc::ptr_eq(&committed, harness.state().session.project()));
    assert_eq!(item_crop(&harness, fixture.video), Some(shape(9.0 / 16.0)));

    open(&mut harness, fixture.video);
    choose(&mut harness, "crop.aspect.2x1");
    click(&mut harness, "crop.close");
    assert!(
        !has_widget(&harness, "crop.apply"),
        "the close button discards"
    );
    assert!(Arc::ptr_eq(&committed, harness.state().session.project()));
    assert_eq!(item_crop(&harness, fixture.video), Some(shape(9.0 / 16.0)));
}

#[test]
fn crop_locked_resize_keeps_the_selected_shape_and_custom_is_free() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    let entry = harness.state().session.project().clone();
    open(&mut harness, fixture.video);

    for (option, expected, aspect) in [
        ("crop.aspect.1x1", 1.0, CropAspect::Square),
        ("crop.aspect.9x16", 9.0 / 16.0, CropAspect::NineSixteen),
    ] {
        choose(&mut harness, option);

        // A corner keeps the opposite corner anchored and the shape.
        let before = working(&harness).crop;
        let corner = point(&harness, before, 1.0, 1.0);
        drag(&mut harness, corner, egui::vec2(-60.0, -60.0));
        assert_locked(&harness, expected, aspect);
        let shrunk = working(&harness).crop;
        assert!(shrunk.width < before.width, "{option} corner shrinks");
        assert!((shrunk.x - before.x).abs() <= 1e-9, "{option} anchor x");
        assert!((shrunk.y - before.y).abs() <= 1e-9, "{option} anchor y");

        // Growing past the frame shrinks to fit and never breaks the ratio.
        let corner = point(&harness, shrunk, 1.0, 1.0);
        drag(&mut harness, corner, egui::vec2(10_000.0, 10_000.0));
        assert_locked(&harness, expected, aspect);
        let grown = working(&harness).crop;
        assert!(grown.x + grown.width <= 1.0 + 1e-9);
        assert!(grown.y + grown.height <= 1.0 + 1e-9);

        // An edge moves one dimension and re-centres the other.
        let centre_y = grown.y + grown.height / 2.0;
        let edge = point(&harness, grown, 1.0, 0.5);
        drag(&mut harness, edge, egui::vec2(-40.0, 0.0));
        assert_locked(&harness, expected, aspect);
        let edged = working(&harness).crop;
        assert!(edged.width < grown.width, "{option} edge shrinks");
        assert!(
            (edged.y + edged.height / 2.0 - centre_y).abs() <= 1e-6,
            "{option} edge keeps the other axis centred"
        );

        // A hostile edge drag floors at the minimum size on both axes.
        let edge = point(&harness, edged, 1.0, 0.5);
        drag(&mut harness, edge, egui::vec2(-10_000.0, 0.0));
        assert_locked(&harness, expected, aspect);
        let tiny = working(&harness).crop;
        assert!(tiny.width >= 0.05 - 1e-9 && tiny.height >= 0.05 - 1e-9);
        assert!(Arc::ptr_eq(&entry, harness.state().session.project()));
        assert_eq!(item_crop(&harness, fixture.video), None);
        click(&mut harness, "crop.reset");
    }

    // Custom frees the shape: the same corner drag changes both axes.
    choose(&mut harness, "crop.aspect.9x16");
    choose(&mut harness, "crop.aspect.custom");
    let free = working(&harness).crop;
    let corner = point(&harness, free, 1.0, 1.0);
    drag(&mut harness, corner, egui::vec2(-60.0, -60.0));
    let changed = working(&harness).crop;
    assert_eq!(working(&harness).aspect, CropAspect::Custom);
    assert!(
        (output_ratio(changed) - output_ratio(free)).abs() > 1e-3,
        "Custom resizes freely"
    );
    assert!(changed.valid());
    assert!(Arc::ptr_eq(&entry, harness.state().session.project()));

    // The chosen shape survives Apply and reopening.
    choose(&mut harness, "crop.aspect.1x1");
    let corner = point(&harness, working(&harness).crop, 1.0, 1.0);
    drag(&mut harness, corner, egui::vec2(-40.0, -40.0));
    assert_locked(&harness, 1.0, CropAspect::Square);
    click(&mut harness, "crop.apply");
    open(&mut harness, fixture.video);
    assert_eq!(working(&harness).aspect, CropAspect::Square);
}
