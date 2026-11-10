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
use engine::{ByteBudget, Command, Session};
use project::{AspectPreset, CanvasSize, Project, ProjectId};
use std::sync::Arc;
use vibecut::{
    Editor, egui,
    preferences::Preferences,
    services::color_picker::palette,
    services::playback::{Files, Playback},
};

fn editor() -> Editor {
    let mut data = Project::empty(ProjectId(1), "Canvas".into())
        .unwrap()
        .into_data();
    data.original_canvas_size = Some(CanvasSize {
        width: 1080,
        height: 1920,
    });
    let session = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Editor::new(session, playback, Preferences::default())
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    harness.run_steps(2);
}

fn enter(harness: &mut Harness<Editor>, id: &str, value: &str) {
    click(harness, id);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(value.to_owned()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
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

fn field_value(harness: &Harness<Editor>, id: &str) -> String {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .value()
        .unwrap()
        .to_owned()
}

fn canvas(harness: &Harness<Editor>) -> project::Canvas {
    harness.state().session.project().data().canvas.clone()
}

#[test]
fn canvas_settings_pick_preset_custom_fps_swatch_reset_and_persist() {
    let mut harness = build(editor());
    harness.run_steps(3);
    assert_eq!(canvas(&harness).preset, Some(AspectPreset::Landscape));
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1920, 1080)
    );

    // A preset commits one history step.
    let before = canvas(&harness);
    let history = harness.state().session.history_len().0;
    click(&mut harness, "inspector.canvas.ratio");
    click(&mut harness, "inspector.canvas.ratio.9x16");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1080, 1920)
    );
    assert_eq!(canvas(&harness).preset, Some(AspectPreset::Portrait));
    assert_eq!(harness.state().session.history_len().0, history + 1);
    assert_ne!(before, canvas(&harness));

    // A custom dimension marks custom mode, remembers the last custom size and
    // commits exactly one step.
    let history = harness.state().session.history_len().0;
    enter(&mut harness, "inspector.canvas.width", "1000");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1000, 1920)
    );
    assert_eq!(canvas(&harness).preset, None);
    assert_eq!(
        canvas(&harness).last_custom_size,
        CanvasSize {
            width: 1000,
            height: 1920
        }
    );
    assert_eq!(harness.state().session.history_len().0, history + 1);

    // A zero dimension is refused without a history step or project change.
    let before = harness.state().session.project().clone();
    let history = harness.state().session.history_len().0;
    enter(&mut harness, "inspector.canvas.height", "0");
    assert_eq!(harness.state().session.project(), &before);
    assert_eq!(harness.state().session.history_len().0, history);
    enter(&mut harness, "inspector.canvas.height", "700");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1000, 700)
    );

    // Odd custom sizes commit the nearest even value (ties up), one at a time,
    // and each field shows the committed value.
    let history = harness.state().session.history_len().0;
    enter(&mut harness, "inspector.canvas.width", "1081");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1082, 700)
    );
    assert_eq!(field_value(&harness, "inspector.canvas.width"), "1082");
    assert_eq!(harness.state().session.history_len().0, history + 1);
    let history = harness.state().session.history_len().0;
    enter(&mut harness, "inspector.canvas.height", "721");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1082, 722)
    );
    assert_eq!(field_value(&harness, "inspector.canvas.height"), "722");
    assert_eq!(harness.state().session.history_len().0, history + 1);
    assert_eq!(
        canvas(&harness).last_custom_size,
        CanvasSize {
            width: 1082,
            height: 722
        }
    );

    // The integer select writes the exact rational, one step per pick.
    let history = harness.state().session.history_len().0;
    click(&mut harness, "inspector.canvas.fps");
    click(&mut harness, "inspector.canvas.fps.60_1");
    assert_eq!(canvas(&harness).fps, time::FrameRate::FPS_60);
    assert_eq!(harness.state().session.history_len().0, history + 1);

    // A fractional source rate keeps its exact rational and its own selected
    // row; picking it neither rounds nor adds history.
    harness
        .state_mut()
        .session
        .apply(Command::SetFps {
            num: 24000,
            den: 1001,
        })
        .unwrap();
    harness.run_steps(3);
    assert_eq!(canvas(&harness).fps, time::FrameRate::FPS_23_976);
    click(&mut harness, "inspector.canvas.fps");
    assert!(has_widget(&harness, "inspector.canvas.fps.current"));
    let history = harness.state().session.history_len().0;
    click(&mut harness, "inspector.canvas.fps.current");
    assert_eq!(canvas(&harness).fps, time::FrameRate::FPS_23_976);
    assert_eq!(harness.state().session.history_len().0, history);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);

    // The background section exposes the whole solid palette and commits one step.
    let history = harness.state().session.history_len().0;
    let swatch = palette::solid(5).unwrap();
    click(&mut harness, "inspector.canvas_swatch.5");
    assert_eq!(canvas(&harness).background, swatch);
    assert_eq!(harness.state().session.history_len().0, history + 1);

    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-46-canvas.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    harness.render().unwrap().save(&path).unwrap();

    // Reset to the retained original size is one step and keeps the preset row.
    let history = harness.state().session.history_len().0;
    click(&mut harness, "inspector.canvas.reset_original");
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1080, 1920)
    );
    assert_eq!(canvas(&harness).preset, Some(AspectPreset::Original));
    assert_eq!(harness.state().session.history_len().0, history + 1);

    // Undo/redo restores the pre-reset canvas exactly.
    harness.state_mut().session.undo().unwrap();
    harness.run_steps(3);
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1082, 722)
    );
    harness.state_mut().session.redo().unwrap();
    harness.run_steps(3);
    assert_eq!(
        (canvas(&harness).width, canvas(&harness).height),
        (1080, 1920)
    );

    // Settings round-trip through persistence with their custom memory.
    let encoded = project::encode(harness.state().session.project(), false).unwrap();
    let loaded = project::decode(&encoded).unwrap().project;
    assert_eq!(&loaded, harness.state().session.project().as_ref());
    assert_eq!(
        loaded.data().canvas.last_custom_size,
        CanvasSize {
            width: 1082,
            height: 722
        }
    );
}
