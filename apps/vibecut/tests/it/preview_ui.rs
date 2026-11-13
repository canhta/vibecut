#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::expect_used
)]
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use engine::{ByteBudget, Session};
use project::{
    Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, TextAlign, TextParams, Track,
    TrackId, TrackKind, TransformField,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    AppCommand, Editor, PreviewZoom, egui,
    i18n::{Locale, Text},
    preferences::{PreferenceStore, Preferences},
    services::playback::{Files, Playback},
    services::preview,
};

fn overlay(id: u64) -> Item {
    Item {
        hidden: false,
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

/// One visible text item whose default position is the lower third of the canvas.
fn text(id: u64) -> Item {
    Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(id),
        source_asset: None,
        source_range: SourceRange::new(Tick::ZERO, Tick(20 * TICKS_PER_SECOND)).unwrap(),
        start: Tick::ZERO,
        content: ItemParams::Text(project::TextParams::new("Hello world")),
    }
}

fn text_project(track_locked: bool, track_hidden: bool) -> Project {
    let mut data = Project::empty(ProjectId(170), "Text".into())
        .unwrap()
        .into_data();
    data.canvas.width = 640;
    data.canvas.height = 360;
    data.canvas.fps = FrameRate::FPS_30;
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(2),
        name: "Text".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: track_hidden,
        locked: track_locked,
        items: vec![text(1)],
    });
    Project::new(data).unwrap()
}

fn styled_text_project(params: TextParams, rotate: f64) -> Project {
    let mut data = Project::empty(ProjectId(174), "Caret".into())
        .unwrap()
        .into_data();
    data.canvas.width = 640;
    data.canvas.height = 360;
    data.canvas.fps = FrameRate::FPS_30;
    let mut item = text(1);
    item.content = ItemParams::Text(params);
    if rotate != 0.0 {
        item.animations
            .set(
                project::AnimationPath::from(TransformField::Rotate.builtin()),
                project::Param::new(project::ParamValue::Scalar(rotate)).unwrap(),
            )
            .unwrap();
    }
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(2),
        name: "Text".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![item],
    });
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
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    h.run_steps(3);
    h
}

/// Double-click needs two presses inside egui's 0.3 s window; the default
/// harness advances 0.25 s per event, so these sessions run at 0.05 s.
fn build_fast(editor: Editor) -> Harness<'static, Editor> {
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_step_dt(0.05)
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    h.run_steps(3);
    h
}

fn canvas_size(h: &Harness<Editor>) -> egui::Vec2 {
    let canvas = &h.state().session.project().data().canvas;
    egui::vec2(canvas.width as f32, canvas.height as f32)
}

/// The fitted canvas rectangle the player displays.
fn canvas_frame(h: &Harness<Editor>) -> egui::Rect {
    let stage = widget(h, "preview.canvas").rect;
    let canvas = canvas_size(h);
    let scale = (stage.width() / canvas.x).min(stage.height() / canvas.y);
    egui::Rect::from_center_size(stage.center(), canvas * scale)
}

/// Screen point at the text item's default (50 %, 84.5 %) canvas position.
fn text_point(h: &Harness<Editor>) -> egui::Pos2 {
    let frame = canvas_frame(h);
    frame.center() + egui::vec2(0.0, (0.845 - 0.5) * frame.height())
}

fn double_click(h: &mut Harness<Editor>, at: egui::Pos2) {
    for _ in 0..2 {
        h.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
    }
    h.run_steps(3);
}

fn text_edit_open(h: &Harness<Editor>) -> bool {
    h.query_all(By::new().predicate(|node| node.author_id() == Some("preview.text.edit")))
        .next()
        .is_some()
}

fn type_into_edit(h: &mut Harness<Editor>, text: &str) {
    h.get(By::new().predicate(|node| node.author_id() == Some("preview.text.edit")))
        .focus();
    h.run_steps(1);
    h.get(By::new().predicate(|node| node.author_id() == Some("preview.text.edit")))
        .type_text(text);
    h.run_steps(2);
}

fn content(project: &Project, item: ItemId) -> String {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .and_then(|candidate| match &candidate.content {
            ItemParams::Text(params) => Some(params.content.as_str().to_owned()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing text item {item:?}"))
}

fn text_item_params(h: &Harness<Editor>) -> TextParams {
    h.state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == ItemId(1))
        .and_then(|candidate| match &candidate.content {
            ItemParams::Text(params) => Some(params.clone()),
            _ => None,
        })
        .expect("text item")
}

fn item_geometry(
    h: &Harness<Editor>,
) -> (render::viewport::ItemGeometry, render::viewport::Viewport) {
    let project = h.state().session.project();
    let renderer = render::Renderer::new(project, render::RenderSettings::from_project(project))
        .expect("renderer");
    let geometry = renderer
        .item_geometry(ItemId(1), Tick::ZERO)
        .expect("geometry")
        .expect("text box");
    let frame = canvas_frame(h);
    let viewport = render::viewport::Viewport::new(
        render::Rect {
            x: f64::from(frame.min.x),
            y: f64::from(frame.min.y),
            width: f64::from(frame.width()),
            height: f64::from(frame.height()),
        },
        [project.data().canvas.width, project.data().canvas.height],
        1.0,
        [0.0, 0.0],
    )
    .expect("viewport");
    (geometry, viewport)
}

/// Text layout, padding, item geometry and viewport used by both expected geometry
/// and pointer targets.
fn text_environment(
    h: &Harness<Editor>,
    content: &str,
) -> (
    text::Layout,
    f32,
    render::viewport::ItemGeometry,
    render::viewport::Viewport,
) {
    let params = text_item_params(h);
    let (geometry, viewport) = item_geometry(h);
    let pad = params.background.as_ref().map_or(0.0, |background| {
        let extent = |value: f32| value.clamp(0.0, project::TextBackground::MAX_EXTENT);
        let offset = |value: f32| {
            value
                .clamp(
                    -project::TextBackground::MAX_OFFSET,
                    project::TextBackground::MAX_OFFSET,
                )
                .abs()
        };
        (extent(background.padding_x) + offset(background.offset_x))
            .max(extent(background.padding_y) + offset(background.offset_y))
            .ceil()
    });
    let style = text::style_name(params.weight, params.italic);
    let bounds = geometry.bounds();
    let layout = text::layout(
        content,
        &text::LayoutOptions {
            family: params.font.as_str(),
            style: &style,
            size: params.font_size,
            max_width: bounds.width as f32 - 2.0 * pad,
            align: match params.align {
                TextAlign::Left => text::Align::Left,
                TextAlign::Center => text::Align::Center,
                TextAlign::Right => text::Align::Right,
            },
            line_height: params.line_height,
            letter_spacing: params.letter_spacing,
            ..Default::default()
        },
    )
    .expect("layout");
    (layout, pad, geometry, viewport)
}

/// Caret and selection from the text crate's own layout and render's item geometry.
fn expected_caret(h: &Harness<Editor>, content: &str) -> vibecut::TextCaret {
    let (layout, pad, geometry, viewport) = text_environment(h, content);
    let count = content.chars().count();
    let start = layout.caret(content, 0);
    let end = layout.caret(content, count);
    let bounds = geometry.bounds();
    let origin = [bounds.x as f32 + pad, bounds.y as f32 + pad];
    let point = |x: f32, y: f32| -> [f32; 2] {
        let [x, y] = geometry
            .point(
                &viewport,
                [f64::from(origin[0] + x), f64::from(origin[1] + y)],
            )
            .expect("point");
        [x as f32, y as f32]
    };
    let mut selection = Vec::new();
    if !content.is_empty() {
        for index in start.line..=end.line {
            let line = &layout.lines[index];
            let left = if index == start.line { start.x } else { line.x };
            let right = if index == end.line {
                end.x
            } else {
                line.x + line.width
            };
            let top = line.baseline - line.ascent;
            let bottom = line.baseline + line.descent;
            selection.push([
                point(left, top),
                point(right, top),
                point(right, bottom),
                point(left, bottom),
            ]);
        }
    }
    vibecut::TextCaret {
        index: count,
        range: 0..count,
        layout: [[end.x, end.top], [end.x, end.bottom]],
        screen: [point(end.x, end.top), point(end.x, end.bottom)],
        selection,
    }
}

/// Screen point at a source character index, offset by logical pixels.
fn text_screen(h: &Harness<Editor>, content: &str, index: usize, offset: egui::Vec2) -> egui::Pos2 {
    let (layout, pad, geometry, viewport) = text_environment(h, content);
    let caret = layout.caret(content, index);
    let bounds = geometry.bounds();
    let [x, y] = geometry
        .point(
            &viewport,
            [
                f64::from(bounds.x as f32 + pad + caret.x + offset.x),
                f64::from(bounds.y as f32 + pad + (caret.top + caret.bottom) / 2.0 + offset.y),
            ],
        )
        .expect("text point");
    egui::pos2(x as f32, y as f32)
}

fn assert_near(actual: [f32; 2], expected: [f32; 2], label: &str) {
    assert!(
        (actual[0] - expected[0]).abs() <= 1.0 && (actual[1] - expected[1]).abs() <= 1.0,
        "{label}: {actual:?} is not within 1 px of {expected:?}"
    );
}

fn assert_caret(actual: &vibecut::TextCaret, expected: &vibecut::TextCaret, label: &str) {
    assert_eq!(actual.index, expected.index, "{label} caret index");
    assert_near(
        actual.layout[0],
        expected.layout[0],
        &format!("{label} layout top"),
    );
    assert_near(
        actual.layout[1],
        expected.layout[1],
        &format!("{label} layout bottom"),
    );
    assert_near(
        actual.screen[0],
        expected.screen[0],
        &format!("{label} screen top"),
    );
    assert_near(
        actual.screen[1],
        expected.screen[1],
        &format!("{label} screen bottom"),
    );
}

fn assert_selection(actual: &vibecut::TextCaret, expected: &vibecut::TextCaret, label: &str) {
    assert_eq!(
        actual.selection.len(),
        expected.selection.len(),
        "{label} selection lines"
    );
    for (index, (actual, expected)) in actual.selection.iter().zip(&expected.selection).enumerate()
    {
        for corner in 0..4 {
            assert_near(
                actual[corner],
                expected[corner],
                &format!("{label} selection line {index} corner {corner}"),
            );
        }
    }
}

fn type_buffer(h: &mut Harness<Editor>, text: &str) {
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            h.key_press(egui::Key::Enter);
            h.run_steps(1);
        }
        if !line.is_empty() {
            type_into_edit(h, line);
        }
    }
}

fn open_text_session(h: &mut Harness<Editor>, text: &str) {
    let point = text_point(h);
    double_click(h, point);
    assert!(text_edit_open(h), "the text session opens");
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    h.run_steps(1);
    if text.is_empty() {
        h.key_press(egui::Key::Backspace);
        h.run_steps(2);
    } else {
        type_buffer(h, text);
    }
    h.run_steps(2);
}

fn click_at(h: &mut Harness<Editor>, at: egui::Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    h.run_steps(1);
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    h.run_steps(2);
}

fn drag_select(h: &mut Harness<Editor>, from: egui::Pos2, to: egui::Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(to));
    h.run_steps(2);
    h.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    h.run_steps(2);
}

fn current_caret(h: &Harness<Editor>) -> vibecut::TextCaret {
    h.state()
        .inspect()
        .unwrap()
        .text_caret
        .expect("open text session")
}

fn click(h: &mut Harness<Editor>, id: &str) {
    if h.query_all(By::new().predicate(|node| node.author_id() == Some(id)))
        .next()
        .is_none()
    {
        let menu = if id.starts_with("preview.zoom.") {
            "preview.zoom"
        } else {
            "preview.menu"
        };
        h.get(By::new().predicate(|node| node.author_id() == Some(menu)))
            .click();
        h.run_steps(3);
    }
    h.get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    h.run_steps(3);
}

/// The AccessKit node id egui assigned to the widget behind one automation id.
fn node_id(h: &Harness<Editor>, id: &str) -> String {
    format!(
        "{:?}",
        h.get(By::new().predicate(|node| node.author_id() == Some(id)))
            .accesskit_node()
            .id()
    )
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

fn select(h: &mut Harness<Editor>, item: ItemId) {
    h.state_mut().select([item].into()).unwrap();
    h.run_steps(3);
}

fn history(h: &Harness<Editor>) -> usize {
    h.state().session.history_len().0
}

fn value(h: &Harness<Editor>, item: ItemId, field: TransformField) -> f64 {
    h.state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .and_then(|candidate| field.value_at(&candidate.animations, Tick::ZERO).ok())
        .unwrap_or_else(|| panic!("missing item {item:?} field {field:?}"))
}

/// The staged candidate's value during a preview gesture, or the committed one.
fn staged(h: &Harness<Editor>, item: ItemId, field: TransformField) -> f64 {
    let project = h
        .state()
        .session
        .preview_project()
        .map(|project| project.as_ref())
        .unwrap_or_else(|| h.state().session.project());
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .and_then(|candidate| field.value_at(&candidate.animations, Tick::ZERO).ok())
        .unwrap_or_else(|| panic!("missing staged item {item:?} field {field:?}"))
}

fn guides(h: &Harness<Editor>) -> Vec<vibecut::PreviewGuide> {
    h.state().inspect().unwrap().preview_guides
}

/// Screen point of a canvas-pixel point at the preview's fitted viewport.
fn screen(viewport: &render::viewport::Viewport, point: [f64; 2]) -> egui::Pos2 {
    let [x, y] = viewport.canvas_to_screen(point).unwrap();
    egui::pos2(x as f32, y as f32)
}

/// The item's box centre and the viewport the player uses for its gestures.
fn box_centre(h: &Harness<Editor>) -> ([f64; 2], render::Rect, render::viewport::Viewport) {
    let (geometry, viewport) = item_geometry(h);
    let bounds = geometry.bounds();
    (
        [
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        ],
        bounds,
        viewport,
    )
}

/// A canvas centre whose box centre and both edges stay at least twice the snap
/// threshold away from every canvas target on both axes.
fn free_spot(bounds: render::Rect, viewport: &render::viewport::Viewport) -> [f64; 2] {
    let threshold = viewport
        .canvas_distance(vibecut::PREVIEW_SNAP_PIXELS)
        .unwrap();
    let clear = |value: f64, half: f64, size: f64| {
        [value - half, value, value + half].iter().all(|edge| {
            [0.0, size / 2.0, size]
                .iter()
                .all(|target| (edge - target).abs() > 2.0 * threshold)
        })
    };
    let x = (0..=640)
        .map(f64::from)
        .find(|value| clear(*value, bounds.width / 2.0, 640.0))
        .expect("a canvas x away from every target");
    let y = (0..=360)
        .map(f64::from)
        .find(|value| clear(*value, bounds.height / 2.0, 360.0))
        .expect("a canvas y away from every target");
    [x, y]
}

fn drag_from(h: &mut Harness<Editor>, start: egui::Pos2, delta: egui::Vec2) {
    h.hover_at(start);
    h.run_steps(1);
    h.drag_at(start);
    h.run_steps(1);
    h.hover_at(start + delta);
    h.run_steps(2);
}

fn drop_at(h: &mut Harness<Editor>, at: egui::Pos2) {
    h.drop_at(at);
    h.run_steps(3);
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
        let label = time::format_timecode(rate.tick_of(90).unwrap(), rate).unwrap();
        let target = time::parse_timecode(&label, rate).unwrap();
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
        click(&mut h, "preview.retry");
        assert!(h.query_by_label("Invalid timecode").is_none(), "{rate:?}");
        assert_eq!(h.state().inspect().unwrap().position, target);
    }
}

#[test]
fn preview_status_control_keeps_one_widget_identity_across_the_retry_flip() {
    // The footer status is one control: it reports the offline state normally
    // and becomes Retry while a failure is showing. The preview fails
    // asynchronously, so a press that starts before the state changes must
    // still resolve to the same widget on release.
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("blocked");
    std::fs::create_dir(&blocked).unwrap();
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    h.state_mut().playback.set_device_available(false);
    h.run_steps(2);
    let offline = node_id(&h, "preview.audio_status");
    assert!(h.state().inspect().unwrap().snapshot_error.is_none());

    assert!(h.state_mut().snapshot_picked_file(blocked).is_err());
    h.run_steps(2);
    let retry = node_id(&h, "preview.retry");
    assert_eq!(
        retry, offline,
        "the status control must keep its widget id when a failure turns it into Retry"
    );

    click(&mut h, "preview.retry");
    assert!(h.state().inspect().unwrap().snapshot_error.is_none());
    assert_eq!(
        node_id(&h, "preview.audio_status"),
        offline,
        "clearing the failure must return the same widget to the offline state"
    );
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
            .dispatch(AppCommand::PreviewZoom(PreviewZoom::Zoom(100.0)))
            .is_err()
    );
    assert!(h.state().inspect().unwrap().preview_fit);
    h.state_mut()
        .dispatch(AppCommand::PreviewZoom(PreviewZoom::Zoom(2.0)))
        .unwrap();
    h.state_mut()
        .dispatch(AppCommand::PreviewZoom(PreviewZoom::Zoom(0.5)))
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
        .dispatch(AppCommand::PreviewZoom(PreviewZoom::Zoom(2.0)))
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
        .dispatch(AppCommand::PreviewZoom(PreviewZoom::Zoom(1.25)))
        .unwrap();
    prefs = h.state().preferences.clone();
    store.save(&prefs).unwrap();
    let restored = store.load().unwrap();
    assert!(!restored.preview_fit);
    assert!((restored.preview_zoom - 1.25).abs() < 1e-6);
    assert_eq!(restored.snapshot_directory, prefs.snapshot_directory);

    // A successful save records its directory in the same preference document.
    h.state_mut()
        .dispatch(AppCommand::PreviewZoom(PreviewZoom::Fit))
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
    h.run_steps(3);
    assert!(
        h.query_by_label(vibecut::i18n::text(
            h.state().preferences.locale,
            Text::SnapshotFailed
        ))
        .is_some()
    );
    let inspect = h.state().inspect().unwrap();
    assert!(matches!(
        inspect.snapshot_error,
        Some(engine::control::ErrorCode::Refused)
    ));
    assert!(inspect.snapshot_saved.is_none());
    assert!(inspect.import_toast.is_none());
    assert_eq!(json(h.state().session.project()), bytes);

    let parent_file = dir.path().join("parent-file");
    std::fs::write(&parent_file, b"preserved").unwrap();
    let error = h
        .state_mut()
        .snapshot_picked_file(parent_file.join("frame.png"))
        .unwrap_err();
    assert!(matches!(
        error,
        vibecut::Error::Engine(engine::control::Error::Edit(engine::EditError::Invalid(
            "snapshot write failed"
        )))
    ));
    let inspect = h.state().inspect().unwrap();
    assert_eq!(inspect.snapshot_error, Some(engine::control::ErrorCode::Io));
    assert!(inspect.snapshot_saved.is_none());
    assert!(inspect.import_toast.is_none());
    assert_eq!(std::fs::read(&parent_file).unwrap(), b"preserved");
    assert_eq!(json(h.state().session.project()), bytes);

    click(&mut h, "preview.retry");
    assert!(h.state().inspect().unwrap().snapshot_error.is_none());
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
    h.state_mut().select([ItemId(1), ItemId(2)].into()).unwrap();
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

fn add_bookmark(editor: &mut Editor, second: i64) {
    editor
        .session
        .apply(engine::Command::BookmarkAdd {
            start: Tick(second * TICKS_PER_SECOND),
            duration: Tick::ZERO,
            note: "Mark".into(),
            color: project::Color([255, 255, 0, 255]),
        })
        .unwrap();
}

#[test]
fn bookmark_jump_preview_menu_shares_lookup_enablement_and_dispatch() {
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    add_bookmark(h.state_mut(), 3);
    add_bookmark(h.state_mut(), 8);
    h.state_mut()
        .dispatch(AppCommand::Seek(Tick::ZERO))
        .unwrap();
    h.run_steps(2);
    h.get(By::new().predicate(|node| node.author_id() == Some("preview.canvas")))
        .click_secondary();
    h.run_steps(3);
    assert!(widget(&h, "preview.menu.jump.next").enabled);
    assert!(!widget(&h, "preview.menu.jump.previous").enabled);
    click(&mut h, "preview.menu.jump.next");
    assert_eq!(
        h.state().playback.position().unwrap(),
        Tick(3 * TICKS_PER_SECOND)
    );

    h.get(By::new().predicate(|node| node.author_id() == Some("preview.canvas")))
        .click_secondary();
    h.run_steps(3);
    click(&mut h, "preview.menu.jump.next");
    assert_eq!(
        h.state().playback.position().unwrap(),
        Tick(8 * TICKS_PER_SECOND)
    );
    h.get(By::new().predicate(|node| node.author_id() == Some("preview.canvas")))
        .click_secondary();
    h.run_steps(3);
    assert!(!widget(&h, "preview.menu.jump.next").enabled);
    assert!(widget(&h, "preview.menu.jump.previous").enabled);
    click(&mut h, "preview.menu.jump.previous");
    assert_eq!(
        h.state().playback.position().unwrap(),
        Tick(3 * TICKS_PER_SECOND)
    );
}

#[test]
fn bookmark_jump_menu_render_screenshot() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&root).unwrap();
    for (locale, name) in [(Locale::En, "en"), (Locale::Vi, "vi")] {
        for pixels in [1.0, 2.0] {
            let mut editor = editor(
                project(FrameRate::FPS_30, 640, 360, false),
                Preferences {
                    locale,
                    ..Preferences::default()
                },
            );
            add_bookmark(&mut editor, 3);
            add_bookmark(&mut editor, 8);
            let mut h = crate::gpu::builder()
                .with_size(egui::vec2(1440.0, 900.0))
                .with_pixels_per_point(pixels)
                .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
            h.run_steps(3);
            h.get(By::new().predicate(|node| node.author_id() == Some("preview.canvas")))
                .click_secondary();
            h.run_steps(3);
            let path = root.join(format!("p1-73-bookmark-nav-preview-{name}-{pixels}x.png"));
            h.render().unwrap().save(&path).unwrap();
            h.key_press(egui::Key::Escape);
            h.run_steps(3);
            let ruler =
                h.get(By::new().predicate(|node| node.author_id() == Some("timeline.ruler")));
            ruler.click_secondary();
            h.run_steps(3);
            let path = root.join(format!("p1-73-bookmark-nav-timeline-{name}-{pixels}x.png"));
            h.render().unwrap().save(&path).unwrap();
            println!("bookmark menu screenshot: {}", path.display());
        }
    }
    std::fs::copy(
        root.join("p1-73-bookmark-nav-preview-en-2x.png"),
        root.join("p1-73-bookmark-nav.png"),
    )
    .unwrap();
}

#[test]
fn edit_after_timecode_seek_resyncs_playback_and_clamps_at_the_new_end() {
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    h.state_mut()
        .dispatch(AppCommand::Seek(Tick(4 * TICKS_PER_SECOND)))
        .unwrap();
    assert_eq!(
        h.state().inspect().unwrap().position,
        Tick(4 * TICKS_PER_SECOND)
    );
    h.state_mut().select([ItemId(1)].into()).unwrap();
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
        locale: vibecut::i18n::Locale::Vi,
        ..Default::default()
    };
    let mut h = build(editor(project(FrameRate::FPS_30, 640, 360, false), prefs));
    assert!(h.query_by_label("Mã thời gian không hợp lệ").is_none());
    // The locale retains stable automation ids for the controls.
    assert!(widget(&h, "preview.timecode").enabled);
    click(&mut h, "preview.menu");
    assert!(widget(&h, "preview.snapshot").enabled);
}

#[test]
fn player_footer_stays_on_one_row_in_both_languages() {
    for locale in [vibecut::i18n::Locale::En, vibecut::i18n::Locale::Vi] {
        let mut h = build(editor(
            project(FrameRate::FPS_30, 640, 360, false),
            Preferences {
                locale,
                ..Default::default()
            },
        ));
        let play = widget(&h, "transport.play").rect;
        let panel = h.state().panels()["preview"];
        assert!((widget(&h, "preview.menu").rect.center().y - panel.top() - 21.0).abs() < 1.0);
        assert!(
            (widget(&h, "preview.fullscreen").rect.center().y - panel.bottom() + 22.0).abs() < 1.0
        );

        let current = widget(&h, "preview.timecode").rect;
        let fullscreen = widget(&h, "preview.fullscreen").rect;
        assert!((play.center().y - current.center().y).abs() < 1.0);
        assert!((play.center().y - fullscreen.center().y).abs() < 1.0);
        assert!(current.right() < play.left());
        assert!(play.right() < widget(&h, "preview.quality").rect.left());
        assert!(widget(&h, "preview.menu").rect.bottom() < widget(&h, "preview.canvas").rect.top());
        click(&mut h, "preview.ratio");
        assert!(
            h.query_by_label(&format!("640 {} 360", '\u{00d7}'))
                .is_some()
        );
        h.key_press(egui::Key::Escape);
        h.state_mut().playback.set_device_available(false);
        h.run_steps(3);
        let audio = widget(&h, "preview.audio_status");
        assert!(!audio.enabled);
        assert!(!audio.rect.intersects(widget(&h, "preview.timecode").rect));
    }
}

#[test]
fn pending_preview_invalidates_presented_readiness_until_the_replacement_is_polled() {
    let mut model = editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while model.playback.frame().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "initial preview timed out"
        );
        model.tick();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // The dedicated preview surface deliberately does not poll: its host owns the clock.
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(628.0, 532.0))
        .build_ui_state(|ui, model: &mut Editor| model.preview_ui(ui), model);
    h.run_steps(3);
    assert!(h.state().presented_frame().is_some());
    h.state_mut().playback.clear().unwrap();
    assert!(h.state().playback.frame().is_none());
    h.run_steps(3);
    assert!(
        h.state().presented_frame().is_none(),
        "loading must not claim the old frame is still presented"
    );
    while h.state().playback.frame().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "replacement preview timed out"
        );
        h.state_mut().tick();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    h.run_steps(3);
    assert!(h.state().presented_frame().is_some());
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
        .dispatch(AppCommand::PreviewZoom(PreviewZoom::Zoom(1.5)))
        .unwrap();
    h.run_steps(3);
    let image = h.render().expect("offscreen render");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-24-preview.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

/// Offscreen evidence for the ticket; wgpu stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-25-preview-edit.png"]
fn preview_transform_handles_render_screenshot() {
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
    h.state_mut().select([ItemId(1)].into()).unwrap();
    h.run_steps(2);
    let corner = widget(&h, "preview.transform.bottom-right").rect.center();
    drag_from(&mut h, corner, egui::vec2(-90.0, -52.0));
    drop_at(&mut h, corner + egui::vec2(-90.0, -52.0));
    let rotate = widget(&h, "preview.transform.rotate").rect.center();
    drag_from(&mut h, rotate, egui::vec2(90.0, -24.0));
    drop_at(&mut h, rotate + egui::vec2(90.0, -24.0));
    h.run_steps(3);
    assert!(widget(&h, "preview.transform.rotate").enabled);
    let image = h.render().expect("offscreen render");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-25-preview-edit.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

#[test]
fn text_contract_has_the_snapshot_and_zoom_labels() {
    assert_eq!(Text::ProjectEmpty.key(), "project_empty");
    assert_eq!(Text::SnapshotSave.key(), "snapshot_save");
    assert_eq!(Text::Fullscreen.key(), "fullscreen");
    assert_eq!(preview::PRESETS, &[25, 50, 75, 100, 150, 200]);
}

fn guarded(track_locked: bool) -> Project {
    let mut data = project(FrameRate::FPS_30, 640, 360, false).into_data();
    let main = &mut data.scene_mut().timeline.main;
    main.locked = track_locked;
    main.hidden = !track_locked;
    Project::new(data).unwrap()
}

#[test]
fn preview_canvas_click_selects_the_item_without_an_edit() {
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    let before = json(h.state().session.project());
    h.get(By::new().predicate(|node| node.author_id() == Some("preview.canvas")))
        .click();
    h.run_steps(3);
    assert!(h.state().selection().contains(&ItemId(1)));
    assert_eq!(history(&h), 0);
    assert_eq!(json(h.state().session.project()), before);
}

#[test]
fn preview_scale_handles_publish_each_corner_and_side_as_one_undo_step() {
    let rows = [
        (
            "preview.transform.top-left",
            egui::vec2(-24.0, -24.0),
            true,
            true,
        ),
        (
            "preview.transform.top-right",
            egui::vec2(24.0, -24.0),
            true,
            true,
        ),
        (
            "preview.transform.right",
            egui::vec2(24.0, 0.0),
            true,
            false,
        ),
        (
            "preview.transform.bottom-right",
            egui::vec2(24.0, 24.0),
            true,
            true,
        ),
        (
            "preview.transform.bottom-left",
            egui::vec2(-24.0, 24.0),
            true,
            true,
        ),
        (
            "preview.transform.left",
            egui::vec2(-24.0, 0.0),
            true,
            false,
        ),
    ];
    for (id, delta, across, down) in rows {
        let mut h = build(editor(
            project(FrameRate::FPS_30, 640, 360, false),
            Preferences::default(),
        ));
        select(&mut h, ItemId(1));
        let before = json(h.state().session.project());
        let start = widget(&h, id).rect.center();
        drag_from(&mut h, start, delta);
        assert_eq!(
            json(h.state().session.project()),
            before,
            "{id} must only stage its candidate"
        );
        assert_eq!(history(&h), 0, "{id} staged no history");
        drop_at(&mut h, start + delta);
        assert_eq!(history(&h), 1, "{id} publishes one undo step");
        assert_eq!(
            value(&h, ItemId(1), TransformField::ScaleX) > 1.0,
            across,
            "{id} scaleX"
        );
        assert_eq!(
            value(&h, ItemId(1), TransformField::ScaleY) > 1.0,
            down,
            "{id} scaleY"
        );
        assert_ne!(json(h.state().session.project()), before, "{id}");
    }
}

#[test]
fn preview_move_and_rotate_publish_one_undo_step_without_direct_mutation() {
    // Move through the canvas body.
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    select(&mut h, ItemId(1));
    let before = json(h.state().session.project());
    let centre = widget(&h, "preview.canvas").rect.center();
    drag_from(&mut h, centre, egui::vec2(40.0, 24.0));
    assert_eq!(
        json(h.state().session.project()),
        before,
        "a staged move must not touch the committed project"
    );
    assert!(h.state().session.gesture_active());
    assert_eq!(history(&h), 0);
    drop_at(&mut h, centre + egui::vec2(40.0, 24.0));
    assert_eq!(history(&h), 1);
    assert_ne!(value(&h, ItemId(1), TransformField::PositionX), 0.0);
    assert_ne!(value(&h, ItemId(1), TransformField::PositionY), 0.0);

    // Rotate through the grip below the box.
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    select(&mut h, ItemId(1));
    let before = json(h.state().session.project());
    let start = widget(&h, "preview.transform.rotate").rect.center();
    drag_from(&mut h, start, egui::vec2(60.0, -40.0));
    assert_eq!(json(h.state().session.project()), before);
    assert_eq!(history(&h), 0);
    drop_at(&mut h, start + egui::vec2(60.0, -40.0));
    assert_eq!(history(&h), 1);
    assert_ne!(value(&h, ItemId(1), TransformField::Rotate), 0.0);
}

#[test]
fn preview_drag_returning_to_its_origin_and_escape_publish_nothing() {
    // A drag that returns exactly to its origin discards the candidate.
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    select(&mut h, ItemId(1));
    let before = json(h.state().session.project());
    let centre = widget(&h, "preview.canvas").rect.center();
    drag_from(&mut h, centre, egui::vec2(48.0, 0.0));
    assert!(h.state().session.gesture_active());
    h.hover_at(centre);
    h.run_steps(2);
    drop_at(&mut h, centre);
    assert_eq!(history(&h), 0);
    assert_eq!(json(h.state().session.project()), before);

    // Escape during the drag discards the staged candidate.
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    select(&mut h, ItemId(1));
    let before = json(h.state().session.project());
    let centre = widget(&h, "preview.canvas").rect.center();
    drag_from(&mut h, centre, egui::vec2(52.0, 28.0));
    assert!(h.state().session.gesture_active());
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().session.gesture_active());
    drop_at(&mut h, centre + egui::vec2(52.0, 28.0));
    assert_eq!(history(&h), 0);
    assert_eq!(json(h.state().session.project()), before);
}

#[test]
fn preview_transform_refuses_hidden_and_locked_items_and_clamps_minimum_scale() {
    for track_locked in [true, false] {
        let mut h = build(editor(guarded(track_locked), Preferences::default()));
        let before = json(h.state().session.project());
        h.get(By::new().predicate(|node| node.author_id() == Some("preview.canvas")))
            .click();
        h.run_steps(3);
        assert!(h.state().selection().is_empty());
        assert_eq!(history(&h), 0);
        assert_eq!(json(h.state().session.project()), before);
        assert!(
            h.query_all(By::new().predicate(|node| {
                node.author_id()
                    .is_some_and(|id| id.starts_with("preview.transform."))
            }))
            .next()
            .is_none(),
            "a locked or hidden item exposes no handles"
        );
    }

    // Crossing the centre clamps to the project minimum instead of flipping.
    let mut h = build(editor(
        project(FrameRate::FPS_30, 640, 360, false),
        Preferences::default(),
    ));
    select(&mut h, ItemId(1));
    let start = widget(&h, "preview.transform.right").rect.center();
    drag_from(&mut h, start, egui::vec2(-2000.0, 0.0));
    drop_at(&mut h, start + egui::vec2(-2000.0, 0.0));
    assert_eq!(history(&h), 1);
    assert!((value(&h, ItemId(1), TransformField::ScaleX) - 0.01).abs() < 1e-9);
}

#[test]
fn preview_move_snaps_to_a_canvas_edge_and_shift_bypasses_the_gesture() {
    let mut h = build(editor(text_project(false, false), Preferences::default()));
    select(&mut h, ItemId(1));
    let (centre, bounds, viewport) = box_centre(&h);
    let snapped = 640.0 - bounds.width / 2.0;
    let start = screen(&viewport, centre);
    let raw = screen(&viewport, [snapped + 3.0, centre[1]]);
    let before = json(h.state().session.project());
    drag_from(&mut h, start, raw - start);
    assert_eq!(
        json(h.state().session.project()),
        before,
        "staging a snapped move never mutates the committed project"
    );
    assert_eq!(history(&h), 0, "staging publishes no history");
    assert!(
        (staged(&h, ItemId(1), TransformField::PositionX) - (snapped - centre[0])).abs() < 1e-6,
        "the box's right edge lands on the canvas right edge"
    );
    assert!(
        guides(&h).iter().any(|guide| {
            guide.axis == vibecut::PreviewAxis::Vertical && (guide.position - 640.0).abs() < 1e-6
        }),
        "an active snap draws its guide: {:?}",
        guides(&h)
    );

    // Shift mid-drag bypasses the snap and removes the guide.
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    h.run_steps(2);
    let bypassed = staged(&h, ItemId(1), TransformField::PositionX);
    assert!(
        (bypassed - (snapped + 3.0 - centre[0])).abs() < 1e-3,
        "Shift bypasses the snap for the rest of the gesture: staged {bypassed} expected {}",
        snapped + 3.0 - centre[0]
    );
    assert!(guides(&h).is_empty());

    // Releasing Shift restores the snap, and the drop publishes one undo step.
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(2);
    assert!(
        (staged(&h, ItemId(1), TransformField::PositionX) - (snapped - centre[0])).abs() < 1e-6
    );
    drop_at(&mut h, raw);
    assert_eq!(history(&h), 1);
    assert!((value(&h, ItemId(1), TransformField::PositionX) - (snapped - centre[0])).abs() < 1e-6);
    assert!(guides(&h).is_empty(), "the guide disappears on release");
}

#[test]
fn preview_scale_snaps_the_dragged_edge_per_axis_and_shift_bypasses() {
    let mut h = build(editor(text_project(false, false), Preferences::default()));
    select(&mut h, ItemId(1));
    let (centre, bounds, viewport) = box_centre(&h);
    let snapped = (640.0 - centre[0]) * 2.0 / bounds.width;
    let start = widget(&h, "preview.transform.right").rect.center();
    let end = screen(&viewport, [637.0, centre[1]]);
    let before = json(h.state().session.project());
    drag_from(&mut h, start, end - start);
    assert_eq!(json(h.state().session.project()), before);
    assert_eq!(history(&h), 0);
    assert!(
        (staged(&h, ItemId(1), TransformField::ScaleX) - snapped).abs() < 1e-6,
        "the dragged right edge lands on the canvas right edge"
    );
    assert_eq!(
        staged(&h, ItemId(1), TransformField::ScaleY),
        1.0,
        "a side handle keeps the other axis"
    );
    assert!(
        guides(&h)
            .iter()
            .any(|guide| guide.axis == vibecut::PreviewAxis::Vertical),
        "the scale snap draws a guide"
    );

    h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    h.run_steps(2);
    let raw = (637.0 - centre[0]) * 2.0 / bounds.width;
    assert!(
        (staged(&h, ItemId(1), TransformField::ScaleX) - raw).abs() < 1e-3,
        "Shift bypasses the scale snap"
    );
    assert!(guides(&h).is_empty());
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(2);
    drop_at(&mut h, end);
    assert_eq!(history(&h), 1);
    assert!((value(&h, ItemId(1), TransformField::ScaleX) - snapped).abs() < 1e-6);
    assert_eq!(value(&h, ItemId(1), TransformField::ScaleY), 1.0);
}

#[test]
fn preview_rotate_snaps_to_quarter_turns_within_five_degrees_and_shift_bypasses() {
    let mut h = build(editor(text_project(false, false), Preferences::default()));
    select(&mut h, ItemId(1));
    let (centre, _, viewport) = box_centre(&h);
    let centre_screen = screen(&viewport, centre);
    let grip = widget(&h, "preview.transform.rotate").rect.center();
    let radius = grip.distance(centre_screen);
    let at = |degrees: f32| {
        let angle = (90.0 + degrees).to_radians();
        centre_screen + egui::vec2(radius * angle.cos(), radius * angle.sin())
    };
    let before = json(h.state().session.project());
    drag_from(&mut h, grip, at(87.0) - grip);
    assert_eq!(json(h.state().session.project()), before);
    assert_eq!(history(&h), 0);
    assert_eq!(
        staged(&h, ItemId(1), TransformField::Rotate),
        90.0,
        "a quarter turn within five degrees snaps"
    );
    assert!(
        guides(&h).iter().any(|guide| {
            guide.axis == vibecut::PreviewAxis::Vertical
                && (guide.position - centre[0]).abs() < 1e-6
        }),
        "the rotation snap draws the item's own axis"
    );

    // Six degrees away is outside the threshold and keeps the raw angle.
    h.hover_at(at(96.0));
    h.run_steps(2);
    let raw = staged(&h, ItemId(1), TransformField::Rotate);
    assert!((raw - 96.0).abs() < 0.5, "outside the threshold, got {raw}");
    assert!(guides(&h).is_empty());

    // Shift bypasses a rotation that would otherwise snap.
    h.hover_at(at(88.0));
    h.run_steps(2);
    assert_eq!(staged(&h, ItemId(1), TransformField::Rotate), 90.0);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
    h.run_steps(2);
    let bypassed = staged(&h, ItemId(1), TransformField::Rotate);
    assert!(
        (bypassed - 88.0).abs() < 0.5,
        "Shift bypasses rotation snapping, got {bypassed}"
    );
    assert!(guides(&h).is_empty());

    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(2);
    drop_at(&mut h, at(88.0));
    assert_eq!(history(&h), 1);
    assert_eq!(value(&h, ItemId(1), TransformField::Rotate), 90.0);
}

#[test]
fn preview_snapping_follows_the_shared_preference_and_shows_no_guide_without_a_candidate() {
    let preferences = Preferences {
        snapping: false,
        ..Default::default()
    };
    let mut h = build(editor(text_project(false, false), preferences));
    select(&mut h, ItemId(1));
    let (centre, bounds, viewport) = box_centre(&h);
    let snapped = 640.0 - bounds.width / 2.0;
    let start = screen(&viewport, centre);
    let raw = screen(&viewport, [snapped + 3.0, centre[1]]);
    drag_from(&mut h, start, raw - start);
    assert!(
        (staged(&h, ItemId(1), TransformField::PositionX) - (snapped + 3.0 - centre[0])).abs()
            < 1e-3,
        "the shared snapping preference turns preview snapping off"
    );
    assert!(guides(&h).is_empty());
    drop_at(&mut h, raw);
    assert_eq!(history(&h), 1);

    // With the preference on, a drag away from every target keeps the raw value
    // and shows no guide.
    let mut h = build(editor(text_project(false, false), Preferences::default()));
    select(&mut h, ItemId(1));
    let (centre, bounds, viewport) = box_centre(&h);
    let free = free_spot(bounds, &viewport);
    let start = screen(&viewport, centre);
    let raw = screen(&viewport, free);
    drag_from(&mut h, start, raw - start);
    assert!(
        (staged(&h, ItemId(1), TransformField::PositionX) - (free[0] - centre[0])).abs() < 1e-3,
        "no candidate inside the threshold leaves the raw position"
    );
    assert!(guides(&h).is_empty());
    drop_at(&mut h, raw);
    assert_eq!(history(&h), 1);
}

/// Offscreen evidence for the ticket; wgpu stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-23-preview-snap.png"]
fn preview_snap_guides_render_screenshot() {
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
    let mut h = build(editor(text_project(false, false), Preferences::default()));
    select(&mut h, ItemId(1));
    let (centre, _, viewport) = box_centre(&h);
    let start = screen(&viewport, centre);
    // Two pixels right and three below the canvas centre: both axes snap.
    let end = screen(&viewport, [322.0, 183.0]);
    drag_from(&mut h, start, end - start);
    assert_eq!(
        guides(&h).len(),
        2,
        "both axes hold a guide: {:?}",
        guides(&h)
    );
    h.run_steps(3);
    let image = h.render().expect("offscreen render");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-23-preview-snap.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

#[derive(Clone, Copy, Debug)]
enum Finish {
    Escape,
    Blur,
    Playback,
}

#[test]
fn preview_inline_text_edit_commits_on_escape_blur_and_playback_with_undo_redo() {
    let rows = [
        (Finish::Escape, "Escape"),
        (Finish::Blur, "blur"),
        (Finish::Playback, "playback"),
    ];
    for (finish, label) in rows {
        let mut h = build_fast(editor(text_project(false, false), Preferences::default()));
        let before = content(h.state().session.project(), ItemId(1));
        let point = text_point(&h);
        double_click(&mut h, point);
        assert!(text_edit_open(&h), "{label} opens the overlay");
        h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
        type_into_edit(&mut h, "Edited");
        assert_eq!(history(&h), 0, "{label} stages no history");
        assert_eq!(
            content(h.state().session.project(), ItemId(1)),
            before,
            "{label} never mutates the committed project directly"
        );
        assert_eq!(
            content(
                h.state().session.preview_project().unwrap().as_ref(),
                ItemId(1)
            ),
            "Edited",
            "{label} stages through the text param command"
        );
        match finish {
            Finish::Escape => {
                h.key_press(egui::Key::Escape);
                h.run_steps(3);
            }
            Finish::Blur => {
                h.get(By::new().predicate(|node| node.author_id() == Some("preview.canvas")))
                    .click();
                h.run_steps(3);
            }
            Finish::Playback => {
                h.state_mut().activate("transport.play").unwrap();
                h.run_steps(3);
                assert!(h.state().playback.playing(), "{label} starts playback");
                h.state_mut().activate("transport.pause").unwrap();
                h.run_steps(2);
            }
        }
        assert!(
            !h.state().session.gesture_active(),
            "{label} closes preview"
        );
        assert!(!text_edit_open(&h), "{label} closes the overlay");
        assert_eq!(history(&h), 1, "{label} publishes one undo step");
        assert_eq!(
            content(h.state().session.project(), ItemId(1)),
            "Edited",
            "{label} commits the buffer"
        );

        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(2);
        assert_eq!(
            content(h.state().session.project(), ItemId(1)),
            before,
            "{label} undo restores the opening content"
        );
        h.state_mut().activate("edit.redo").unwrap();
        h.run_steps(2);
        assert_eq!(
            content(h.state().session.project(), ItemId(1)),
            "Edited",
            "{label} redo restores the edit"
        );
        assert_eq!(history(&h), 1, "{label} stays one step");
    }
}

#[test]
fn preview_inline_text_edit_enter_inserts_a_newline_and_empty_content_is_allowed() {
    // Enter inserts a newline, never commits.
    let mut h = build_fast(editor(text_project(false, false), Preferences::default()));
    let point = text_point(&h);
    double_click(&mut h, point);
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    type_into_edit(&mut h, "A");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    type_into_edit(&mut h, "B");
    assert!(text_edit_open(&h), "Enter keeps the session open");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(content(h.state().session.project(), ItemId(1)), "A\nB");
    assert_eq!(history(&h), 1);

    // Emptying the buffer is allowed, keeps the caret anchored and commits.
    let mut h = build_fast(editor(text_project(false, false), Preferences::default()));
    let before = content(h.state().session.project(), ItemId(1));
    let point = text_point(&h);
    double_click(&mut h, point);
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    h.key_press(egui::Key::Backspace);
    h.run_steps(2);
    assert!(text_edit_open(&h), "an empty buffer keeps the session open");
    assert_eq!(
        content(
            h.state().session.preview_project().unwrap().as_ref(),
            ItemId(1)
        ),
        ""
    );
    assert_eq!(
        content(h.state().session.project(), ItemId(1)),
        before,
        "the committed project is untouched while staging empty content"
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(content(h.state().session.project(), ItemId(1)), "");
    assert_eq!(history(&h), 1);
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(2);
    assert_eq!(content(h.state().session.project(), ItemId(1)), before);
}

#[test]
fn preview_inline_text_edit_refuses_locked_and_hidden_items() {
    for (track_locked, track_hidden) in [(true, false), (false, true)] {
        let mut h = build_fast(editor(
            text_project(track_locked, track_hidden),
            Preferences::default(),
        ));
        let before = json(h.state().session.project());
        let point = text_point(&h);
        double_click(&mut h, point);
        assert!(
            !text_edit_open(&h),
            "a locked or hidden item exposes no inline editor"
        );
        assert!(h.state().selection().is_empty());
        assert_eq!(history(&h), 0);
        assert_eq!(json(h.state().session.project()), before);
    }
}

#[test]
fn preview_inline_text_caret_and_selection_follow_the_rendered_layout() {
    type Row = (&'static str, fn(&mut TextParams), f64, &'static str);
    let rows: &[Row] = &[
        (
            "custom font",
            |params| params.font = "Noto Serif".into(),
            0.0,
            "VibeCut",
        ),
        (
            "letter spacing",
            |params| params.letter_spacing = 8.0,
            0.0,
            "AVAVA",
        ),
        ("two lines", |_| {}, 0.0, "First\nSecond"),
        (
            "centre align",
            |params| params.align = TextAlign::Center,
            0.0,
            "Centre",
        ),
        (
            "right align",
            |params| params.align = TextAlign::Right,
            0.0,
            "Right",
        ),
        (
            "thirty degree rotation",
            |params| {
                params.font = "Noto Serif".into();
                params.letter_spacing = 3.0;
            },
            30.0,
            "Rotate",
        ),
        (
            "wrapped line",
            |_| {},
            0.0,
            "The quick brown fox jumps over the lazy dog",
        ),
        ("empty buffer", |_| {}, 0.0, ""),
    ];
    for (label, setup, rotate, typed) in rows {
        let mut params = TextParams::new("Hello world");
        setup(&mut params);
        let mut h = build_fast(editor(
            styled_text_project(params, *rotate),
            Preferences::default(),
        ));
        let point = text_point(&h);
        double_click(&mut h, point);
        assert!(text_edit_open(&h), "{label} opens the overlay");
        h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
        h.run_steps(1);
        if typed.is_empty() {
            h.key_press(egui::Key::Backspace);
            h.run_steps(2);
        } else {
            type_buffer(&mut h, typed);
        }
        // Collapse the whole buffer to its end: arrow keys and paste leave the
        // caret elsewhere depending on the host input path.
        h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
        h.key_press(egui::Key::ArrowRight);
        h.run_steps(2);
        let caret = h
            .state()
            .inspect()
            .unwrap()
            .text_caret
            .unwrap_or_else(|| panic!("{label} exposes the caret"));
        assert_caret(&caret, &expected_caret(&h, typed), label);

        // The same layout drives the selection after selecting the whole buffer.
        h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
        h.run_steps(2);
        let selected = h
            .state()
            .inspect()
            .unwrap()
            .text_caret
            .unwrap_or_else(|| panic!("{label} exposes the selection"));
        let expected = expected_caret(&h, typed);
        assert_caret(&selected, &expected, label);
        assert_selection(&selected, &expected, label);
    }

    // Pointer input goes through the inverse layout query and render geometry,
    // not egui's provisional galley.
    let mut params = TextParams::new("First\nSecond");
    params.font = "Noto Serif".into();
    params.font_size = 28.0;
    let mut h = build_fast(editor(
        styled_text_project(params, 0.0),
        Preferences::default(),
    ));
    let content = "First\nSecond";
    open_text_session(&mut h, content);
    for (label, index) in [("first line", 2usize), ("second line", 9usize)] {
        let point = text_screen(&h, content, index, egui::vec2(0.0, 0.0));
        click_at(&mut h, point);
        let caret = current_caret(&h);
        assert_eq!(caret.index, index, "{label} click index");
        assert_eq!(caret.range, index..index, "{label} click range");
    }

    // A click past the first line's end clamps to that line's end, not the next line.
    let past_end = text_screen(&h, content, 5, egui::vec2(60.0, 0.0));
    click_at(&mut h, past_end);
    let caret = current_caret(&h);
    assert_eq!(caret.index, 5, "past line end index");
    assert_eq!(caret.range, 5..5, "past line end range");

    // A drag from the first line extends the selection across the line break.
    let from = text_screen(&h, content, 2, egui::vec2(0.0, 0.0));
    let to = text_screen(&h, content, 8, egui::vec2(0.0, 0.0));
    drag_select(&mut h, from, to);
    let caret = current_caret(&h);
    assert_eq!(caret.index, 8, "drag selection primary index");
    assert_eq!(caret.range, 2..8, "drag selection range");

    // A double click selects the word under the pointer using Unicode boundaries.
    let word = text_screen(&h, content, 7, egui::vec2(0.0, 0.0));
    double_click(&mut h, word);
    let caret = current_caret(&h);
    assert_eq!(caret.index, 12, "double click word end");
    assert_eq!(caret.range, 6..12, "double click word range");

    // Rotation is inverted through the same item geometry before layout lookup.
    let mut params = TextParams::new("Rotate");
    params.font = "Noto Serif".into();
    let mut h = build_fast(editor(
        styled_text_project(params, 30.0),
        Preferences::default(),
    ));
    open_text_session(&mut h, "Rotate");
    let point = text_screen(&h, "Rotate", 3, egui::vec2(0.0, 0.0));
    click_at(&mut h, point);
    let caret = current_caret(&h);
    assert_eq!(caret.index, 3, "rotated click index");
    assert_eq!(caret.range, 3..3, "rotated click range");
}

/// Offscreen evidence for the ticket; wgpu stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-174-caret.png"]
fn preview_inline_text_caret_renders_screenshot() {
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
    let mut params = TextParams::new("Rotated caret");
    params.font = "Noto Serif".into();
    params.letter_spacing = 3.0;
    let mut h = build_fast(editor(
        styled_text_project(params, 30.0),
        Preferences::default(),
    ));
    let point = text_point(&h);
    double_click(&mut h, point);
    assert!(text_edit_open(&h));
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    type_into_edit(&mut h, "Rotated caret");
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    h.run_steps(2);
    let mut rendered = false;
    for _ in 0..80 {
        h.run_steps(1);
        if h.state().playback.frame().is_some() {
            rendered = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(rendered, "the edited preview frame arrives");
    assert!(
        h.get(By::new().predicate(|node| node.author_id() == Some("preview.text.edit")))
            .is_focused(),
        "the capture keeps the caret and selection visible"
    );
    let image = h.render().expect("offscreen render");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-174-caret.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

/// Offscreen evidence for the ticket; wgpu stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-25-text-edit.png"]
fn preview_inline_text_edit_renders_screenshot() {
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
    let mut h = build_fast(editor(text_project(false, false), Preferences::default()));
    let point = text_point(&h);
    double_click(&mut h, point);
    assert!(text_edit_open(&h));
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    type_into_edit(&mut h, "Inline heading");
    // Select the buffer so the capture shows the selection and caret treatment.
    h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    h.run_steps(2);
    // The edited preview renders on a worker: give it real time before capturing.
    let mut rendered = false;
    for _ in 0..80 {
        h.run_steps(1);
        if h.state().playback.frame().is_some() {
            rendered = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(rendered, "the edited preview frame arrives");
    assert!(
        h.get(By::new().predicate(|node| node.author_id() == Some("preview.text.edit")))
            .is_focused(),
        "the screenshot keeps the caret and selection visible"
    );
    let image = h.render().expect("offscreen render");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-25-text-edit.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}
