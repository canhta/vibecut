use super::super::Failure;
use super::super::application::*;
use super::super::fixture::*;
use crate::subtitles_fixture;
use egui_kittest::kittest::{By, Queryable};
use engine::ByteBudget;
use engine::Command;
use media::AssetData;
use media::AssetId;
use media::SourceAsset;
use media::StreamKind;
use project::ItemParams;
use project::VideoParams;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use time::FrameRate;
use time::SourceRange;
use time::Tick;
use vibecut::AppCommand;
use vibecut::Editor;
use vibecut::PreviewZoom;
use vibecut::egui;
use vibecut::i18n::Locale;
use vibecut::model_manager::ModelManager;
use vibecut::services::playback::Files;
use vibecut::services::playback::Playback;

pub(super) fn editor_empty(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    Ok(harness_sized(
        App::Editor(Box::new(empty_editor(theme, Locale::En)?)),
        scale,
        [1727.0, 1029.0],
    ))
}

pub(super) fn editor_frame_state(
    theme: Theme,
    scale: f32,
    state: &str,
) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(
        theme,
        if state == "vi" {
            Locale::Vi
        } else {
            Locale::En
        },
    )?;
    if state == "rename" {
        editor.activate("project.rename")?;
    }
    let size = if state == "minimum" {
        [1280.0, 800.0]
    } else {
        [1727.0, 1029.0]
    };
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, size);
    app.run_steps(3);
    if state == "menu" {
        app.get(By::new().predicate(|node| node.author_id() == Some("window.menu")))
            .click();
        app.run_steps(2);
    }
    Ok(app)
}

pub(super) fn captions_scene(
    theme: Theme,
    scale: f32,
    locale: Locale,
    filled: bool,
) -> Result<AppHarness, Failure> {
    let mut editor = if filled {
        subtitles_fixture::editor("vi")
    } else {
        empty_editor(theme, locale)?
    };
    editor.preferences = preferences(theme, locale);
    editor.preferences.assets_tab = vibecut::panels::AssetsTab::Captions;
    editor.prepare_subtitles()?;
    if filled {
        let mut panel = editor.subtitles.take().ok_or("captions missing")?;
        panel.tick(&mut editor.session)?;
        panel.edit_text(
            &mut editor.session,
            &"cue-0".into(),
            captions::LanguageTag::parse("en")?,
            "Hello, and welcome to the caption editor.",
        )?;
        panel.select_cue(&editor.session, 0, false, false)?;
        editor.subtitles = Some(panel);
        editor
            .playback
            .seek(Tick(2 * SECOND), false, editor.session.project().clone())?;
    }
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1280.0, 800.0]);
    app.run_steps(4);
    Ok(app)
}
pub(super) fn models(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    models_locale(theme, scale, Locale::En)
}

pub(super) fn models_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    models_locale(theme, scale, Locale::Vi)
}

pub(super) fn models_locale(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let cache = tempfile::tempdir()?;
    let budget = ByteBudget::new(8 << 20)?;
    let (artifacts, _) = engine::cache::ArtifactCache::open(cache.path(), budget.clone(), 8)?;
    let transfers = engine::models::ModelTransfers::new(
        artifacts,
        Arc::new(NoTransfer),
        Arc::new(engine::models::SystemModelIo),
    );
    let manager = ModelManager::with_transfers(budget, ai::models::catalog()?, transfers)?;
    let mut editor = empty_editor(theme, locale)?;
    editor.set_model_manager(manager);
    editor.open_models()?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1280.0, 800.0]);
    app.state_mut()._model_cache = Some(cache);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.run_steps(1);
        if !app
            .state()
            .inspect()?
            .models
            .as_ref()
            .is_some_and(|models| models.checking)
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err("model check did not finish before the audit deadline".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(4);
    Ok(app)
}
pub(super) fn player_editor(theme: Theme, empty: bool) -> Result<Editor, Failure> {
    let mut session = empty_session("Interview")?;
    let rate = FrameRate::FPS_30;
    let duration = rate.tick_of((3600 + 18 * 60 + 28) * 30 + 6)?;
    session.apply(Command::RenameScene {
        scene: session.project().data().current_scene_id,
        name: "Timeline 01".into(),
    })?;
    if !empty {
        let mut asset: AssetData = source_asset(
            1,
            StreamKind::Video,
            std::env::temp_dir().join("player-source.mp4"),
        )?
        .into();
        asset.probe.duration = Some(duration);
        asset.probe.streams = vec![source_stream(StreamKind::Video, duration)?];
        session.apply(Command::AddAsset {
            asset: SourceAsset::try_from(asset)?,
        })?;
        let main = session.project().data().scene().timeline.main.id;
        insert(
            &mut session,
            main,
            Tick::ZERO,
            SourceRange::new(Tick::ZERO, duration)?,
            Some(AssetId(1)),
            ItemParams::Video(VideoParams {
                source_audio_enabled: true,
                volume: 0.0,
                crop: None,
                muted: false,
                fades: Default::default(),
            }),
        )?;
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    if !empty {
        playback.seek(rate.tick_of(11 * 30 + 7)?, false, session.project().clone())?;
    }
    let mut editor = Editor::new(session, playback, preferences(theme, Locale::En));
    if !empty {
        wait_for_frame(&mut editor)?;
    }
    Ok(editor)
}
pub(super) fn player_scene(theme: Theme, scale: f32, empty: bool) -> Result<AppHarness, Failure> {
    player_variant(theme, scale, if empty { "empty" } else { "media" })
}
pub(super) fn player_variant(
    theme: Theme,
    scale: f32,
    variant: &str,
) -> Result<AppHarness, Failure> {
    let mut editor = player_editor(theme, variant == "empty")?;
    if variant == "offline" {
        editor.playback.set_device_available(false);
    }
    if variant == "vi" {
        editor.preferences.locale = Locale::Vi;
    }
    if variant == "zoomed" {
        editor.dispatch(AppCommand::PreviewZoom(PreviewZoom::Zoom(1.25)))?;
    }
    if matches!(variant, "loading" | "failed") {
        editor.playback = Playback::new(
            &editor.session,
            ByteBudget::new(256 << 20)?,
            if variant == "loading" {
                Arc::new(LoadingSources)
            } else {
                Arc::new(Files)
            },
        )?;
        editor.playback.set_device_available(true);
    }
    if variant == "fullscreen" {
        editor.activate("preview.fullscreen")?;
    }
    let app_root = if variant == "fullscreen" {
        App::Editor(Box::new(editor))
    } else {
        App::Player(Box::new(editor))
    };
    let mut app = harness_sized(app_root, scale, [1840.0, 1142.0]);
    app.run_steps(3);
    if !matches!(variant, "empty" | "loading" | "failed") {
        wait_for_presented(&mut app)?;
    }
    if variant == "failed" {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app
            .state()
            .inspect()?
            .widgets
            .iter()
            .any(|widget| widget.id == "preview.retry")
        {
            if Instant::now() >= deadline {
                return Err("player failure audit timed out".into());
            }
            app.run_steps(1);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    if let Some(id) = match variant {
        "menu" => Some("preview.menu"),
        "zoom" => Some("preview.zoom"),
        "ratio" => Some("preview.ratio"),
        "quality" => Some("preview.quality"),
        "timecode" | "invalid" => Some("preview.timecode"),
        _ => None,
    } {
        app.get(By::new().predicate(|node| node.author_id() == Some(id)))
            .click();
        app.run_steps(3);
    }
    if variant == "invalid" {
        app.get(By::new().predicate(|node| node.author_id() == Some("preview.timecode")))
            .type_text("99:99:99:99");
        app.key_press(egui::Key::Enter);
        app.run_steps(3);
    }
    Ok(app)
}
pub(super) fn editor_media(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (editor, _) = media_editor(theme, Locale::En)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1727.0, 1029.0]);
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    Ok(app)
}
pub(super) fn export_dialog(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    editor.session.apply(Command::SetCanvas {
        width: 360,
        height: 640,
        preset: None,
    })?;
    editor.activate("file.export")?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1727.0, 1029.0]);
    wait_for_presented(&mut app)?;
    app.run_steps(4);
    Ok(app)
}

pub(super) fn export_resolution(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = export_dialog(theme, scale)?;
    if let App::Editor(editor) = &mut app.state_mut().app {
        editor.preferences.export.output.resolution = Some([1440, 2560]);
    }
    app.run_steps(2);
    app.get(By::new().predicate(|node| node.author_id() == Some("export.resolution")))
        .click();
    app.run_steps(20);
    Ok(app)
}

pub(super) fn import_progress(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(theme, Locale::En)?;
    editor.set_import_probe(Arc::new(ProgressProbe::new()))?;
    editor.drop_files(vec![
        std::env::temp_dir().join("one.mp4"),
        std::env::temp_dir().join("two.mp4"),
        std::env::temp_dir().join("three.mp4"),
    ])?;
    let mut app = app_editor(editor, scale);
    for _ in 0..500 {
        app.run_steps(1);
        let inspection = app.state().inspect()?;
        if inspection.import_completed > 0 {
            break;
        }
        if !inspection.importing {
            return Err("import finished before the audit could sample progress".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    let inspection = app.state().inspect()?;
    if !inspection.importing {
        return Err("import finished before the audit could render progress".into());
    }
    Ok(app)
}
