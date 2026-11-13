//! Builds properties, inspector, import, text, close-prompt scenes.
use super::super::Failure;
use super::super::application::{
    App, AppHarness, SECOND, Theme, harness, harness_sized, preferences,
};
use super::super::fixture::{
    CompletedProbe, app_editor, empty_editor, insert, media_editor, range, wait_for_frame,
    wait_for_presented, widget_rect,
};
use egui_kittest::kittest::{By, Queryable};
use engine::ByteBudget;
use engine::Command;
use engine::ItemParamPatch;
use engine::Session;
use media::AssetId;
use project::Color;
use project::ItemParams;
use project::OverlayParams;
use project::Project;
use project::ProjectId;
use project::TextParams;
use project::TrackKind;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use time::Tick;
use vibecut::Editor;
use vibecut::app::Workspace;
use vibecut::i18n::Locale;
use vibecut::services::playback::Files;
use vibecut::services::playback::Playback;

pub(super) fn inspector_speed(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, item) = media_editor(theme, Locale::En)?;
    editor.select([item].into_iter().collect()).unwrap();
    editor.session.apply(Command::SetSpeed {
        items: vec![item],
        speed: time::Speed::from_f64(1.5)?,
        maintain_pitch: true,
    })?;
    editor
        .preferences
        .set("properties_tabs", serde_json::json!({"video":"speed"}))?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    widget_rect(&app, "inspector.speed.pitch")?;
    Ok(app)
}

pub(super) fn inspector_audio_locale(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, locale)?;
    let audio = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .audio
        .first()
        .and_then(|track| track.items.first())
        .ok_or("audio item")?
        .id;
    editor.select([audio].into_iter().collect()).unwrap();
    editor.set_shared_field("audio_gain", "-3")?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

pub(super) fn inspector_audio(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_audio_locale(theme, scale, Locale::En)
}

pub(super) fn inspector_audio_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_audio_locale(theme, scale, Locale::Vi)
}
pub(super) fn inspector_mixed_locale(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, locale)?;
    let media: Vec<_> = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .filter(|item| matches!(item.content, ItemParams::Video(_) | ItemParams::Audio(_)))
        .map(|item| item.id)
        .collect();
    editor.session.apply(Command::Delete { items: media })?;
    let (text_track, first) = {
        let scene = editor.session.project().data().scene();
        let track = scene
            .timeline
            .iter()
            .find(|track| track.kind == TrackKind::Text)
            .ok_or("text track")?;
        (track.id, track.items.first().ok_or("first text item")?.id)
    };
    let second = insert(
        &mut editor.session,
        text_track,
        Tick(6 * SECOND),
        range(6, 8)?,
        None,
        ItemParams::Text(TextParams {
            content: "Khác".into(),
            font_size: 24.0,
            color: Color([20, 30, 40, 255]),
            ..Default::default()
        }),
    )?;
    let main = editor.session.project().data().scene().timeline.main.id;
    let background = insert(
        &mut editor.session,
        main,
        Tick::ZERO,
        range(0, 10)?,
        None,
        ItemParams::Overlay(OverlayParams {
            color: Color([35, 45, 65, 255]),
        }),
    )?;
    editor
        .select([first, second, background].into_iter().collect())
        .unwrap();
    // Setup edits use Session directly, so refresh the preview snapshot before
    // accepting a frame from the populated-editor fixture.
    let project = editor.session.project().clone();
    editor.playback.seek(Tick(3 * SECOND), false, project)?;
    wait_for_frame(&mut editor)?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

pub(super) fn inspector_mixed(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_mixed_locale(theme, scale, Locale::En)
}

pub(crate) fn inspector_mixed_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_mixed_locale(theme, scale, Locale::Vi)
}

pub(super) fn inspector_blend_locale(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let mut data = Project::empty(ProjectId(44), "Blend".into())?.into_data();
    data.canvas.width = 64;
    data.canvas.height = 64;
    data.canvas.background = Color([120, 160, 200, 255]);
    let mut session = Session::new(Project::new(data)?, 10)?;
    let main = session.project().data().scene().timeline.main.id;
    let item = insert(
        &mut session,
        main,
        Tick::ZERO,
        range(0, 10)?,
        None,
        ItemParams::Overlay(OverlayParams {
            color: Color([200, 80, 120, 128]),
        }),
    )?;
    session.apply(Command::SetItemField {
        item,
        patch: ItemParamPatch::BlendMode(project::BlendMode::Multiply),
    })?;
    session.apply(Command::SetItemField {
        item,
        patch: ItemParamPatch::Opacity(0.5),
    })?;
    let mut playback = Playback::new(&session, ByteBudget::new(64 << 20)?, Arc::new(Files))?;
    playback.set_device_available(true);
    let mut prefs = preferences(theme, locale);
    prefs.set(
        "properties_tabs",
        serde_json::json!({"graphic": "transform"}),
    )?;
    prefs.set("assets_tab", serde_json::json!("effects"))?;
    let mut editor = Editor::new(session, playback, prefs);
    editor.select([item].into()).unwrap();
    wait_for_frame(&mut editor)?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

pub(super) fn inspector_blend(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_blend_locale(theme, scale, Locale::En)
}

pub(super) fn inspector_blend_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_blend_locale(theme, scale, Locale::Vi)
}
/// Properties scenes render at the reference window size so crops align.
pub(super) const CAPTURE_WINDOW: [f32; 2] = [1728.0, 1030.0];

/// The capture's column split (34 / 28 %), so the panel is 486 px wide.
pub(super) fn capture_columns(editor: &mut Editor) -> Result<(), Failure> {
    editor
        .preferences
        .set("panels", serde_json::json!([0.337, 0.28125, 0.45]))?;
    Ok(())
}

pub(super) fn inspector_video_tab(
    theme: Theme,
    scale: f32,
    tab: &str,
) -> Result<AppHarness, Failure> {
    let (mut editor, item) = media_editor(
        theme,
        if tab.ends_with("-vi") {
            Locale::Vi
        } else {
            Locale::En
        },
    )?;
    capture_columns(&mut editor)?;
    let tab = tab.strip_suffix("-vi").unwrap_or(tab);
    let kinds_spatial = matches!(
        tab,
        "effects-streak" | "effects-edge-lines" | "effects-relief"
    );
    let colour_picker = tab == "effects-colours-picker";
    let mixer_bottom = tab == "effects-mixer-bottom";
    let tab = if tab.starts_with("effects") {
        let kinds: &[&str] = if tab == "effects-streak" {
            &["directional_blur"]
        } else if tab == "effects-edge-lines" {
            &["find_edges"]
        } else if tab == "effects-relief" {
            &["color_emboss"]
        } else if tab == "effects-threshold" {
            &["extract"]
        } else if tab.starts_with("effects-mixer") {
            &["channel_mix"]
        } else if tab == "effects-tonal" {
            &["black_white", "levels", "gamma_correction", "posterize"]
        } else if tab.starts_with("effects-colours") {
            &["tint", "strobe"]
        } else {
            &["invert", "color_pass", "color_replace"]
        };
        for kind in kinds {
            editor.session.apply(Command::AddEffect {
                item,
                effect: if *kind == "directional_blur" {
                    project::effect_definition(kind)?
                        .instance(*kind)
                        .with("length", project::EffectValue::Scalar(125.0))?
                        .with("direction", project::EffectValue::Scalar(45.0))?
                } else {
                    project::effect_definition(kind)?.instance(*kind)
                },
            })?;
        }
        "effects"
    } else {
        tab
    };
    editor.select([item].into_iter().collect()).unwrap();
    editor
        .preferences
        .set("properties_tabs", serde_json::json!({"video": tab}))?;
    let size = if kinds_spatial {
        [1727.0, 1029.0]
    } else {
        CAPTURE_WINDOW
    };
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, size);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    if mixer_bottom {
        app.get(
            By::new().predicate(|node| node.author_id() == Some("effects.channel_mix.param.bc")),
        )
        .scroll_to_me();
        app.run_steps(3);
    }
    if colour_picker {
        app.get(By::new().predicate(|node| node.author_id() == Some("effects.tint.param.black")))
            .click();
        app.run_steps(3);
    }
    Ok(app)
}

pub(super) fn inspector_video(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_video_tab(theme, scale, "transform")
}

pub(super) fn inspector_video_blend(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_video_tab(theme, scale, "transform")
}

pub(super) fn inspector_project(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    capture_columns(&mut editor)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, CAPTURE_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

pub(super) fn inspector_text(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    let item = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Text(_)))
        .ok_or("text item")?
        .id;
    capture_columns(&mut editor)?;
    editor.select([item].into_iter().collect()).unwrap();
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, CAPTURE_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

pub(super) fn inspector_transform(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = inspector_blend(theme, scale)?;
    if let App::Editor(editor) = &mut app.state_mut().app {
        editor.preferences.set(
            "properties_tabs",
            serde_json::json!({"graphic":"transform"}),
        )?;
    }
    app.run_steps(3);
    Ok(app)
}
pub(super) fn inspector_color(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = inspector_blend(theme, scale)?;
    if let App::Editor(editor) = &mut app.state_mut().app {
        editor
            .preferences
            .set("properties_tabs", serde_json::json!({"graphic":"graphic"}))?;
        let item = editor
            .selection()
            .first()
            .copied()
            .ok_or("colour audit item")?;
        editor.open_color_picker(vibecut::ColorTarget::GraphicFill(item))?;
        editor.set_eyedropper(vibecut::services::color_picker::NativeEyedropper::unavailable())?;
    }
    app.run_steps(3);
    Ok(app)
}
pub(super) fn text_item_locale(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, locale)?;
    let item = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Text(_)))
        .ok_or("text item")?
        .id;
    editor.select([item].into_iter().collect()).unwrap();
    for (field, value) in [
        ("text_content", "VibeCut · Tiếng Việt"),
        ("text_font", "Noto Serif"),
        ("text_background", "#000000A0"),
        ("text_position_y", "70"),
    ] {
        editor.set_shared_field(field, value)?;
    }
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1440.0, 1440.0]);
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    Ok(app)
}

pub(super) fn text_item(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    text_item_locale(theme, scale, Locale::En)
}

pub(super) fn text_item_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    text_item_locale(theme, scale, Locale::Vi)
}
pub(super) fn import_complete(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(theme, locale)?;
    editor.set_import_probe(Arc::new(CompletedProbe))?;
    editor.drop_files(vec![
        std::env::temp_dir().join("video-a.mp4"),
        std::env::temp_dir().join("bad-clip.mp4"),
        std::env::temp_dir().join("audio-a.wav"),
    ])?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while editor.importing() {
        editor.tick();
        if Instant::now() >= deadline {
            return Err("import did not finish".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    editor.select_media([AssetId(1)]);
    let mut app = app_editor(editor, scale);
    app.run_steps(3);
    Ok(app)
}

pub(super) fn import_complete_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    import_complete(theme, scale, Locale::Vi)
}

pub(super) fn close_prompt_locale(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let (editor, _) = media_editor(theme, locale)?;
    let directory = tempfile::tempdir()?;
    let mut workspace = Workspace::new(
        directory.path().join("recovery"),
        preferences(theme, locale),
        None,
    )?;
    let id = workspace.register_editor(Box::new(editor));
    workspace
        .windows
        .get_mut(&id)
        .ok_or("close-prompt project window")?
        .request_close();
    let mut app = harness(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
    );
    app.run_steps(4);
    Ok(app)
}

pub(super) fn close_prompt(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    close_prompt_locale(theme, scale, Locale::En)
}

pub(super) fn close_prompt_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    close_prompt_locale(theme, scale, Locale::Vi)
}
