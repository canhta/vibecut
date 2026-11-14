//! Builds editor, player, captions, model-manager, export, import-progress scenes.
use super::super::Failure;
use super::super::application::{App, AppHarness, SECOND, Theme, harness_sized, preferences};
use super::super::fixture::{
    LoadingSources, NoTransfer, ProgressProbe, SyntheticSources, app_editor, empty_editor,
    empty_session, insert, media_editor, source_asset, source_stream, wait_for_frame,
    wait_for_presented,
};
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

/// A backend that has a key and refuses to open a model: the audit only paints the consent
/// sheet, so nothing is ever sent.
struct KeyedBackend;
impl vibecut::AgentBackend for KeyedBackend {
    fn has_key(&self, _: agent::Provider) -> bool {
        true
    }
    fn open(
        &self,
        _: agent::Provider,
    ) -> Result<Box<dyn ai::text::conversation::ChatModel>, agent::Error> {
        Err(agent::Error::Chat(ai::Error::Auth))
    }
}

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
        if matches!(state, "vi" | "agent-vi") {
            Locale::Vi
        } else {
            Locale::En
        },
    )?;
    if state == "rename" {
        editor.activate("project.rename")?;
    }
    if state.starts_with("agent") {
        editor.set_agent_backend(Arc::new(KeyedBackend));
        editor.open_agent();
    }
    let size = if state == "minimum" {
        vibecut::preferences::WINDOW_MIN
    } else {
        [1727.0, 1029.0]
    };
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, size);
    app.run_steps(3);
    if state == "agent-expanded" {
        app.get(By::new().predicate(|node| node.author_id() == Some("agent.expand")))
            .click();
        app.run_steps(3);
    }
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
    let editor = if filled {
        subtitles_fixture::editor("vi")
    } else {
        empty_editor(theme, locale)?
    };
    captions_editor(theme, scale, locale, editor, filled, false)
}

pub(super) fn caption_word_scene(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let cues = subtitles_fixture::verified_cues();
    let editor = subtitles_fixture::editor_with_cues("vi", cues);
    captions_editor(theme, scale, locale, editor, true, true)
}

/// The transcript review after Identify pauses and repeats finished: gap card, tabs, rows,
/// mini timeline and footer.
pub(super) fn captions_review_scene(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let mut editor = subtitles_fixture::editor_with_cues("en", subtitles_fixture::review_cues());
    editor.preferences = preferences(theme, locale);
    editor.preferences.assets_tab = vibecut::AssetsTab::Captions;
    editor.prepare_subtitles()?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1280.0, 800.0]);
    app.run_steps(4);
    app.get(By::new().predicate(|node| node.author_id() == Some("subtitles.review.analyze")))
        .click();
    let deadline = Instant::now() + Duration::from_secs(20);
    while app
        .query_all(By::new().predicate(|node| node.author_id() == Some("subtitles.review.tab.all")))
        .next()
        .is_none()
    {
        if Instant::now() > deadline {
            return Err("the transcript analysis did not finish".into());
        }
        app.run_steps(1);
    }
    app.run_steps(3);
    Ok(app)
}

fn captions_editor(
    theme: Theme,
    scale: f32,
    locale: Locale,
    mut editor: Editor,
    filled: bool,
    highlighted: bool,
) -> Result<AppHarness, Failure> {
    editor.preferences = preferences(theme, locale);
    if highlighted {
        editor.preferences.subtitles.bilingual = true;
    }
    editor.preferences.assets_tab = vibecut::AssetsTab::Captions;
    editor.prepare_subtitles()?;
    if filled {
        editor
            .with_subtitles(
                |panel, session, _| -> Result<(), Box<dyn std::error::Error>> {
                    panel.tick(session)?;
                    panel.edit_text(
                        session,
                        &"cue-0".into(),
                        captions::LanguageTag::parse("en")?,
                        "Hello, and welcome to the caption editor.",
                    )?;
                    panel.select_cue(session, 0, false, false)?;
                    Ok(())
                },
            )
            .ok_or("captions missing")??;
        editor
            .playback
            .seek(Tick(2 * SECOND), false, editor.session.project().clone())?;
    }
    if highlighted {
        editor.activate("subtitles.style.word_highlight")?;
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
    let manager =
        ModelManager::with_transfers(budget, ai::foundation::models::catalog()?, transfers)?;
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
                ..VideoParams::plain()
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
    if variant == "guides" {
        // The reference shot 100216: rulers on, one horizontal and one vertical guide.
        editor.activate("preview.rulers")?;
        editor.activate("preview.guides.add_horizontal")?;
        editor.activate("preview.guides.add_vertical")?;
    }
    if variant == "scope" {
        // The reference's strip: parade, waveform and vectorscope under the video
        // (capcut-desktop-2026-11-14f/scopes/04-player-with-scopes.png).
        editor.activate("preview.scopes")?;
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
/// The media tile menu open on the asset a clip uses, so the tile also shows its Added badge
/// (reference 103259).
pub(super) fn editor_media_menu(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = editor_media(theme, scale)?;
    app.get(By::new().predicate(|node| node.author_id() == Some("media.asset.1")))
        .click_secondary();
    app.run_steps(3);
    Ok(app)
}
/// The media page with one folder tile beside the loose asset.
pub(super) fn editor_media_folder(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    editor.session.apply(Command::CreateMediaFolder {
        name: "Day one".into(),
        assets: vec![AssetId(2)],
    })?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1727.0, 1029.0]);
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    Ok(app)
}
/// The Favorites page of the Media sub-navigation with nothing starred (reference 111010).
pub(super) fn editor_media_favorites(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = editor_media(theme, scale)?;
    app.get(By::new().predicate(|node| node.author_id() == Some("assets.subnav.favorites")))
        .click();
    app.run_steps(3);
    Ok(app)
}
/// The Transitions tab with an applicable cut selected, at the measured editor size.
pub(super) fn editor_transitions(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, first) = media_editor(theme, Locale::En)?;
    editor.preferences.assets_tab = vibecut::AssetsTab::Transitions;
    let second = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .main
        .items
        .iter()
        .find(|item| item.id != first && item.source_asset.is_some())
        .map(|item| item.id)
        .ok_or("second video item")?;
    editor.select([first, second].into_iter().collect())?;
    // The transition panel is measured against the 1727x1029 desktop capture.
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1727.0, 1029.0]);
    wait_for_presented(&mut app)?;
    app.run_steps(4);
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

/// The export dialog after clicking the listed widgets in order, one frame burst each, then
/// scrolled so the `reveal` widget (the changed section) is in view.
fn export_clicks(
    theme: Theme,
    scale: f32,
    ids: &[&str],
    reveal: Option<&str>,
) -> Result<AppHarness, Failure> {
    let mut app = export_dialog(theme, scale)?;
    for id in ids {
        // A widget scrolled out of the form's view takes no click; bring it in first.
        app.get(By::new().predicate(|node| node.author_id() == Some(*id)))
            .scroll_to_me();
        app.run_steps(6);
        app.get(By::new().predicate(|node| node.author_id() == Some(*id)))
            .click();
        app.run_steps(12);
    }
    if let Some(id) = reveal {
        app.get(By::new().predicate(|node| node.author_id() == Some(id)))
            .scroll_to_me();
        app.run_steps(6);
    }
    Ok(app)
}

/// Audio and GIF ticked, Video unticked, scrolled to the Audio Format row (reference
/// 105736: Video unticked with Audio and GIF ticked).
pub(super) fn export_audio_only(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    export_clicks(
        theme,
        scale,
        &[
            "export.audio.include",
            "export.gif.include",
            "export.video.include",
        ],
        Some("export.audio.format"),
    )
}

/// Video, Audio and GIF ticked together, scrolled to the GIF Resolution row (reference
/// 105532: GIF ticked beside the default Video and Audio).
pub(super) fn export_gif(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    export_clicks(
        theme,
        scale,
        &["export.audio.include", "export.gif.include"],
        Some("export.gif.resolution"),
    )
}

/// Bit rate set to Custom, showing the Kbps field (reference 105630).
pub(super) fn export_custom_bit_rate(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    export_clicks(
        theme,
        scale,
        &["export.bitrate", "export.bitrate.custom"],
        None,
    )
}

/// The export dialog over a stored project whose cover is set, scrolled to the Color space row
/// (reference 105340: the cover with its Edit cover chip, Rec. 709 SDR under the Video rows).
pub(super) fn export_cover(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    editor.session.apply(Command::SetCanvas {
        width: 360,
        height: 640,
        preset: None,
    })?;
    let project = (**editor.session.project()).clone();
    let mut autosave = engine::recovery::Autosave::new(
        &std::env::temp_dir().join("vibecut-audit-cover"),
        engine::Session::new(project, 50)?,
        ByteBudget::new(64 << 20)?,
        engine::footage::uncorrected(),
        Arc::new(engine::jobs::SystemClock::default()),
        Default::default(),
    )?;
    // A fixed gradient stands in for the captured frame: the audit paints the dialog, not the
    // renderer, and the media of this fixture is synthetic.
    let (width, height) = (72_u32, 128_u32);
    let rgba = (0..height)
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                let down = y as f32 / (height - 1) as f32;
                let across = x as f32 / (width - 1) as f32;
                [
                    (40.0 + 60.0 * down) as u8,
                    (150.0 - 70.0 * down) as u8,
                    (160.0 - 40.0 * across) as u8,
                    255,
                ]
            })
        })
        .collect();
    autosave.session_mut().restore_metadata(project::Metadata {
        thumbnail: project::Thumbnail::Ready {
            width,
            height,
            rgba,
        },
        ..Default::default()
    })?;
    editor.session = vibecut::services::lifecycle::EditorSession::Persistent(Box::new(autosave));
    editor.activate("file.export")?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1727.0, 1029.0]);
    wait_for_presented(&mut app)?;
    app.run_steps(4);
    app.get(By::new().predicate(|node| node.author_id() == Some("export.color_space")))
        .scroll_to_me();
    app.run_steps(6);
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

/// The About dialog as the header menu opens it, or its third-party licence page.
pub(super) fn about_dialog(
    theme: Theme,
    scale: f32,
    locale: Locale,
    licences: bool,
) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(theme, locale)?;
    editor.activate(if licences {
        "app.licences"
    } else {
        "app.about"
    })?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1280.0, 800.0]);
    app.run_steps(6);
    Ok(app)
}

/// The Settings modal as Window / Settings opens it (Draft tab), or on the Edit tab.
pub(super) fn settings_dialog(
    theme: Theme,
    scale: f32,
    locale: Locale,
    edit_tab: bool,
) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(theme, locale)?;
    editor.activate("preferences.open")?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, [1280.0, 800.0]);
    app.run_steps(6);
    if edit_tab {
        app.get(By::new().predicate(|n| n.author_id() == Some("settings.tab.edit")))
            .click();
        app.run_steps(4);
    }
    Ok(app)
}
