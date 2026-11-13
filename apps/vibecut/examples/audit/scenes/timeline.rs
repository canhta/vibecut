//! Builds timeline, playhead, track, scene, placement audit scenes.
use super::super::Failure;
use super::super::application::{
    App, AppHarness, SECOND, Theme, WINDOW, harness_sized, preferences,
};
use super::super::fixture::{
    SyntheticSources, app_editor, empty_editor, empty_session, insert, media_editor, range,
    source_asset, track, wait_for_frame, wait_for_presented, widget_rect,
};
use egui_kittest::kittest::{By, Queryable};
use engine::ByteBudget;
use engine::Command;
use engine::CreatedId;
use media::AssetId;
use media::StreamKind;
use project::AudioParams;
use project::Color;
use project::ItemParams;
use project::OverlayParams;
use project::TextParams;
use project::TrackKind;
use project::VideoParams;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use time::Tick;
use vibecut::Editor;
use vibecut::egui;
use vibecut::i18n::Locale;
use vibecut::services::playback::Files;
use vibecut::services::playback::Playback;

pub(super) fn timeline_zoom(theme: Theme, scale: f32, zoom: f64) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    editor.session.apply(Command::SetViewState {
        zoom,
        scroll: 0.0,
        playhead: Tick::ZERO,
    })?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}
pub(super) fn timeline_selection_locale(
    theme: Theme,
    scale: f32,
    locale: Locale,
) -> Result<AppHarness, Failure> {
    let (editor, item) = media_editor(theme, locale)?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    let id = format!("timeline.item.{}", item.0);
    let node = app
        .query_all(By::new().predicate(|node| node.author_id() == Some(id.as_str())))
        .next()
        .ok_or_else(|| format!("missing automation id {id}"))?;
    node.click();
    app.run_steps(3);
    Ok(app)
}

pub(super) fn timeline_selection(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    timeline_selection_locale(theme, scale, Locale::En)
}

pub(super) fn timeline_selection_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    timeline_selection_locale(theme, scale, Locale::Vi)
}

pub(super) const TRACKS_WINDOW: [f32; 2] = [1728.0, 1200.0];
/// The measured reference editor window, so crops compare at equal logical size.
pub(super) const EDITOR_WINDOW: [f32; 2] = [1728.0, 1030.0];

/// One long clip on the main track, like the single-track reference capture.
pub(super) fn timeline_main(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    timeline_main_sized(theme, scale, EDITOR_WINDOW)
}

pub(super) fn timeline_main_sized(
    theme: Theme,
    scale: f32,
    window: [f32; 2],
) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Timeline")?;
    for (id, kind) in [(1, StreamKind::Video), (2, StreamKind::Audio)] {
        session.apply(Command::AddAsset {
            asset: source_asset(
                id,
                kind,
                std::env::temp_dir().join(format!("source-{id}.mp4")),
            )?,
        })?;
    }
    let main = session.project().data().scene().timeline.main.id;
    for index in 0..4 {
        insert(
            &mut session,
            main,
            Tick(index * 10 * SECOND),
            range(0, 10)?,
            Some(AssetId(1)),
            ItemParams::Video(VideoParams {
                fades: Default::default(),
                source_audio_enabled: true,
                volume: 0.0,
                muted: false,
                crop: None,
                cleanup: Default::default(),
                background: project::ClipBackground::None,
                reduce_noise: false,
            }),
        )?;
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    // 11.24 s puts the playhead 54 px left of the 12 s label, as in the capture.
    let playhead = Tick((11.24 * SECOND as f64) as i64);
    session.apply(Command::SetViewState {
        zoom: 1.7875,
        scroll: 575.0,
        playhead,
    })?;
    let mut editor = Editor::new(session, playback, preferences(theme, Locale::En));
    let project = editor.session.project().clone();
    editor.playback.seek(playhead, false, project)?;
    wait_for_frame(&mut editor)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, window);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

/// The preview axis holding a pointer time over the lanes.
pub(super) fn preview_axis(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = timeline_main(theme, scale)?;
    hover_preview_axis(&mut app)?;
    Ok(app)
}

/// The same state at the cited web capture's 1440×900 logical size.
pub(super) fn preview_axis_reference(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = timeline_main_sized(theme, scale, WINDOW)?;
    hover_preview_axis(&mut app)?;
    Ok(app)
}

pub(super) fn hover_preview_axis(app: &mut AppHarness) -> Result<(), Failure> {
    let ruler = widget_rect(app, "timeline.ruler")?;
    let zoom = app.state().inspect()?.zoom;
    app.hover_at(egui::pos2(ruler.left() + 4.0 * zoom, ruler.center().y));
    app.run_steps(2);
    wait_for_presented(app)?;
    Ok(())
}

/// Layer-end and global-tail evidence at the web reference's logical size.
pub(super) fn preview_axis_ends(theme: Theme, scale: f32, at: Tick) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Preview axis")?;
    let main = session.project().data().scene().timeline.main.id;
    insert(
        &mut session,
        main,
        Tick::ZERO,
        range(0, 2)?,
        None,
        ItemParams::Overlay(OverlayParams {
            color: Color([255, 0, 0, 255]),
        }),
    )?;
    let overlay = track(&mut session, "Overlay", TrackKind::Video)?;
    let blue = insert(
        &mut session,
        overlay,
        Tick::ZERO,
        range(0, 4)?,
        None,
        ItemParams::Overlay(OverlayParams {
            color: Color([0, 0, 255, 255]),
        }),
    )?;
    let playhead = Tick(SECOND);
    session.apply(Command::SetViewState {
        zoom: 1.0,
        scroll: 0.0,
        playhead,
    })?;
    let mut playback = Playback::new(&session, ByteBudget::new(64 << 20)?, Arc::new(Files))?;
    let mut settings = render::RenderSettings::from_project(session.project());
    settings.placements.insert(
        blue,
        render::Placement {
            destination: render::Rect {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
            ..Default::default()
        },
    );
    playback.set_render_settings(settings)?;
    playback.seek(playhead, false, session.project().clone())?;
    let editor = Editor::new(session, playback, preferences(theme, Locale::En));
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, WINDOW);
    app.run_steps(3);
    let ruler = widget_rect(&app, "timeline.ruler")?;
    let zoom = app.state().inspect()?.zoom;
    app.hover_at(egui::pos2(
        ruler.left() + at.seconds() as f32 * zoom,
        ruler.center().y,
    ));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.run_steps(1);
        if app.state().presented_frame() == Some(at) {
            break;
        }
        if Instant::now() >= deadline {
            return Err("axis audit frame timeout".into());
        }
        std::thread::yield_now();
    }
    Ok(app)
}

/// Populated multi-track timeline (video, audio, text) at the reference size.
pub(super) fn timeline_multi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (editor, _) = media_editor(theme, Locale::En)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, EDITOR_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

// New bottom-corner design: the reference captures only panel duration fields.
pub(super) fn timeline_fades(theme: Theme, scale: f32, state: &str) -> Result<AppHarness, Failure> {
    let locale = if state == "menu" {
        Locale::Vi
    } else {
        Locale::En
    };
    let (mut editor, _) = media_editor(theme, locale)?;
    let audio = editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Audio(_)))
        .ok_or("missing audio clip")?
        .id;
    for (edge, seconds, curve) in [
        (project::FadeEdge::Out, 3, project::FadeCurve::Exponential),
        (project::FadeEdge::In, 20, project::FadeCurve::EqualPower),
    ] {
        editor.session.apply(Command::SetFade {
            item: audio,
            edge,
            duration: Tick(seconds * SECOND),
            curve,
        })?;
    }
    if state == "split" {
        editor.session.apply(Command::SplitAt {
            item: audio,
            at: Tick(2 * SECOND),
        })?;
    }
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, EDITOR_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    let handle = format!("timeline.item.{}.fade_in", audio.0);
    if state == "menu" {
        app.get(By::new().predicate(|node| node.author_id() == Some(handle.as_str())))
            .click_secondary();
    } else if state == "cancelled" {
        let entry = app
            .state()
            .session()
            .ok_or("missing editor")?
            .project()
            .clone();
        let from = widget_rect(&app, &handle)?.center();
        let zoom = app.state().inspect()?.zoom;
        app.drag_at(from);
        app.run_steps(1);
        app.hover_at(egui::pos2(from.x - zoom, from.y));
        app.run_steps(2);
        app.key_press(egui::Key::Escape);
        app.run_steps(2);
        app.drop_at(from);
        app.run_steps(2);
        assert_eq!(
            app.state().session().ok_or("missing editor")?.project(),
            &entry
        );
    }
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

pub(super) fn tracks(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Tracks")?;
    for kind in [
        TrackKind::Video,
        TrackKind::Text,
        TrackKind::Sticker,
        TrackKind::Effect,
        TrackKind::Filter,
        TrackKind::Adjustment,
        TrackKind::Audio,
    ] {
        track(&mut session, &format!("{kind:?}"), kind)?;
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    let editor = Editor::new(session, playback, preferences(theme, Locale::En));
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, TRACKS_WINDOW);
    app.run_steps(4);
    Ok(app)
}

pub(super) fn scene_create(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = app_editor(empty_editor(theme, Locale::En)?, scale);
    app.run_steps(3);
    for id in ["scene.open", "scene.create"] {
        app.query_all(By::new().predicate(|node| node.author_id() == Some(id)))
            .next()
            .ok_or_else(|| format!("missing automation id {id}"))?
            .click();
        app.run_steps(3);
    }
    Ok(app)
}

pub(super) fn scene_rename(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut app = scene_create(theme, scale)?;
    let created = app
        .state()
        .session()
        .ok_or("scene rename requires an editor")?
        .project()
        .data()
        .current_scene_id;
    let row = format!("scene.row.{}", created.0);
    app.query_all(By::new().predicate(|node| node.author_id() == Some(row.as_str())))
        .next()
        .ok_or("missing created scene row")?
        .click_secondary();
    app.run_steps(3);
    let rename = format!("scene.rename.{}", created.0);
    app.query_all(By::new().predicate(|node| node.author_id() == Some(rename.as_str())))
        .next()
        .ok_or("missing scene rename action")?
        .click();
    app.run_steps(3);
    Ok(app)
}

pub(super) fn scene_delete(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Scenes")?;
    let mut ids = Vec::new();
    for name in ["Opening", "Closing"] {
        let created = session.apply(Command::CreateScene { name: name.into() })?;
        if let Some(CreatedId::Scene(id)) = created.created.first() {
            ids.push(*id);
        }
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    let editor = Editor::new(session, playback, preferences(theme, Locale::En));
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, TRACKS_WINDOW);
    app.run_steps(3);
    for id in ["scene.open", "scene.select"] {
        app.query_all(By::new().predicate(|node| node.author_id() == Some(id)))
            .next()
            .ok_or_else(|| format!("missing automation id {id}"))?
            .click();
        app.run_steps(3);
    }
    for id in ids {
        let row = format!("scene.row.{}", id.0);
        app.query_all(By::new().predicate(|node| node.author_id() == Some(row.as_str())))
            .next()
            .ok_or_else(|| format!("missing automation id {row}"))?
            .click();
        app.run_steps(2);
    }
    app.query_all(By::new().predicate(|node| node.author_id() == Some("scene.delete")))
        .next()
        .ok_or("missing automation id scene.delete")?
        .click();
    app.run_steps(3);
    if app
        .query_by_label("Delete selected scenes? You can undo this action.")
        .is_none()
    {
        return Err("scene delete confirmation did not open".into());
    }
    Ok(app)
}

pub(super) fn placement(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Placement")?;
    for (id, kind) in [(1, StreamKind::Video), (2, StreamKind::Audio)] {
        session.apply(Command::AddAsset {
            asset: source_asset(
                id,
                kind,
                std::env::temp_dir().join(format!("source-{id}.mp4")),
            )?,
        })?;
    }
    let video = track(&mut session, "Video", TrackKind::Video)?;
    let moving = insert(
        &mut session,
        video,
        Tick::ZERO,
        range(0, 6)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            crop: None,
            muted: false,
            cleanup: Default::default(),
            background: project::ClipBackground::None,
            reduce_noise: false,
        }),
    )?;
    insert(
        &mut session,
        video,
        Tick(6 * SECOND),
        range(6, 10)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            crop: None,
            volume: 0.0,
            muted: false,
            cleanup: Default::default(),
            background: project::ClipBackground::None,
            reduce_noise: false,
        }),
    )?;
    let audio = track(&mut session, "Audio", TrackKind::Audio)?;
    insert(
        &mut session,
        audio,
        Tick::ZERO,
        range(0, 10)?,
        Some(AssetId(2)),
        ItemParams::Audio(AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
            cleanup: Default::default(),
        }),
    )?;
    let text = track(&mut session, "Text", TrackKind::Text)?;
    insert(
        &mut session,
        text,
        Tick(2 * SECOND),
        range(2, 5)?,
        None,
        ItemParams::Text(TextParams {
            content: "Chapter one".into(),
            font_size: 48.0,
            color: Color([255, 255, 255, 255]),
            ..Default::default()
        }),
    )?;
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, preferences(theme, Locale::En));
    let project = editor.session.project().clone();
    editor.playback.seek(Tick(3 * SECOND), false, project)?;
    wait_for_frame(&mut editor)?;
    let mut app = harness_sized(App::Editor(Box::new(editor)), scale, TRACKS_WINDOW);
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    let body = widget_rect(&app, &format!("timeline.item.{}", moving.0))?;
    let header = widget_rect(&app, &format!("track.{}.header", video.0))?;
    let before = app
        .state()
        .session()
        .ok_or("editor session")?
        .project()
        .data()
        .scene()
        .timeline
        .track_count();
    let target = egui::pos2(body.center().x, header.bottom() - 1.0);
    app.drag_at(body.center());
    app.run_steps(1);
    app.hover_at(target);
    app.run_steps(2);
    app.drop_at(target);
    app.run_steps(2);
    let after = app
        .state()
        .session()
        .ok_or("editor session")?
        .project()
        .data()
        .scene()
        .timeline
        .track_count();
    if after <= before {
        return Err("placement drag did not create a track".into());
    }
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    Ok(app)
}
