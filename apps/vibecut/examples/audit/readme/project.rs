//! Captures README editor frames, edits, snapshots through the real Editor path.
use super::super::Failure;
use super::super::application::{App, AppHarness, SECOND, Theme, preferences};
use super::super::fixture::{insert, range, settle, track};
use super::super::report::opaque;
use super::source::ReadmeClock;
use super::source::hex;
use super::source::sha256_bytes;
use captions::Cue;
use captions::CueText;
use captions::CueTrack;
use captions::LanguageTag;
use engine::ByteBudget;
use engine::Session;
use project::AudioParams;
use project::CaptionParams;
use project::Color;
use project::ItemId;
use project::ItemParams;
use project::Project;
use project::ProjectId;
use project::TextParams;
use project::TrackKind;
use project::VideoParams;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use time::FrameRate;
use time::Tick;
use vibecut::Editor;
use vibecut::i18n::Locale;
use vibecut::services::playback::Files;
use vibecut::services::playback::Playback;

pub(super) fn readme_editor(app: &mut AppHarness) -> Result<&mut Editor, Failure> {
    match &mut app.state_mut().app {
        App::Editor(editor) => Ok(editor),
        _ => Err("README capture requires an editor".into()),
    }
}

pub(super) fn readme_frame(app: &mut AppHarness, at: Tick) -> Result<(), Failure> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let ready = {
            let editor = readme_editor(app)?;
            editor.playback.poll(Instant::now())?;
            editor.playback.frame().is_some_and(|frame| {
                frame.generation == editor.playback.generation() && frame.at == at
            })
        };
        if ready {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("README preview frame {} timed out", at.0).into());
        }
        app.run_steps(1);
        std::thread::sleep(Duration::from_millis(2));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.run_steps(1);
        if app.state().presented_frame() == Some(at) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("README preview frame {} not presented", at.0).into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    settle(app)?;
    if app.state().presented_frame() != Some(at) {
        return Err("settled README frame changed".into());
    }
    Ok(())
}

pub(super) fn readme_seek(app: &mut AppHarness, at: Tick) -> Result<(), Failure> {
    {
        let editor = readme_editor(app)?;
        let project = editor.session.project().clone();
        editor.playback.seek(at, false, project)?;
    }
    readme_frame(app, at)
}

pub(super) fn readme_capture(app: &mut AppHarness, path: &Path) -> Result<(), Failure> {
    app.run_steps(2);
    let image = app.render().map_err(std::io::Error::other)?;
    let image = opaque(image, vibecut::window_color(true));
    image.save(path)?;
    Ok(())
}

pub(super) fn readme_frame_digest(app: &mut AppHarness) -> Result<String, Failure> {
    let editor = readme_editor(app)?;
    let frame = editor.playback.frame().ok_or("missing README frame")?;
    let mut hasher = Sha256::new();
    for pixel in &frame.image.pixels {
        hasher.update(pixel.to_array());
    }
    Ok(hex(&hasher.finalize()))
}

pub(super) fn readme_project(source: &Path) -> Result<(Editor, ItemId, ItemId), Failure> {
    let mut data = Project::empty(ProjectId(245), "README media".into())?.into_data();
    data.canvas.width = 1280;
    data.canvas.height = 720;
    data.canvas.fps = FrameRate::FPS_30;
    let session = Session::new(Project::new(data)?, 100)?;
    let mut preferences = preferences(Theme::Dark, Locale::En);
    preferences.assets_tab = vibecut::AssetsTab::Media;
    preferences.subtitles.source = "vi".into();
    preferences.subtitles.target = "en".into();
    preferences.subtitles.bilingual = true;
    let playback = Playback::new(&session, ByteBudget::new(1 << 30)?, Arc::new(Files))?;
    let mut editor = Editor::new(session, playback, preferences);
    let clock = Arc::new(ReadmeClock(AtomicU64::new(0)));
    editor.set_clock(clock.clone());
    editor.drop_files(vec![source.to_owned()])?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while editor.importing() {
        editor.tick();
        if Instant::now() >= deadline {
            return Err("README media import timed out".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    clock.0.store(6_000, Ordering::SeqCst);
    editor.tick();
    let canonical = std::fs::canonicalize(source)?;
    let asset = editor
        .session
        .project()
        .data()
        .assets
        .iter()
        .find(|asset| asset.path() == canonical)
        .cloned()
        .ok_or("README media import produced no asset")?;
    let probe = asset.probe();
    let video = probe
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref())
        .ok_or("README media has no video probe")?;
    if (video.width, video.height) != (1280, 720) {
        return Err(format!("README source probe {}x{}", video.width, video.height).into());
    }
    let asset_id = asset.id();
    let main = editor.session.project().data().scene().timeline.main.id;
    let first = insert(
        &mut editor.session,
        main,
        Tick::ZERO,
        range(0, 6)?,
        Some(asset_id),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            crop: None,
            cleanup: Default::default(),
        }),
    )?;
    let second = insert(
        &mut editor.session,
        main,
        Tick(6 * SECOND),
        range(6, 10)?,
        Some(asset_id),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            crop: None,
            cleanup: Default::default(),
        }),
    )?;
    let audio = track(&mut editor.session, "Audio", TrackKind::Audio)?;
    insert(
        &mut editor.session,
        audio,
        Tick::ZERO,
        range(0, 10)?,
        Some(asset_id),
        ItemParams::Audio(AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
            cleanup: Default::default(),
        }),
    )?;
    let text = track(&mut editor.session, "Text", TrackKind::Text)?;
    let chapter = insert(
        &mut editor.session,
        text,
        Tick(2 * SECOND),
        range(2, 6)?,
        None,
        ItemParams::Text(TextParams {
            content: "Chapter one".into(),
            font_size: 52.0,
            color: Color([255, 255, 255, 255]),
            ..Default::default()
        }),
    )?;
    // The chapter title keeps the 70 % height the readme frame was captured at: 20 % of the
    // 720 px canvas below the centre, as a Transform Y offset.
    editor.session.apply(engine::Command::SetItemField {
        item: chapter,
        patch: engine::ItemParamPatch::PositionY(720.0 * 0.2),
    })?;
    let caption_text = |text: &str| CueText {
        text: text.into(),
        edited: false,
        translation_ref: None,
    };
    let cue = Cue {
        word_timing: captions::CueWordTiming::Unverified,
        id: "readme-cue".into(),
        range: range(2, 6)?,
        source_lang: LanguageTag::parse("vi")?,
        texts: [
            (LanguageTag::parse("vi")?, caption_text("Chào bạn")),
            (LanguageTag::parse("en")?, caption_text("Hello friend")),
        ]
        .into_iter()
        .collect(),
        style: None,
    };
    let captions = track(&mut editor.session, "Captions", TrackKind::Text)?;
    insert(
        &mut editor.session,
        captions,
        Tick::ZERO,
        range(0, 10)?,
        Some(asset_id),
        ItemParams::Caption(CaptionParams {
            emphasis: Default::default(),
            word_highlight: false,
            preset: None,
            overrides: captions::CaptionOverrides::default(),
            cues: CueTrack {
                styles: Vec::new(),
                cues: vec![cue],
            },
        }),
    )?;
    let mut settings = render::RenderSettings::from_project(editor.session.project());
    settings.caption_languages = vec![LanguageTag::parse("vi")?, LanguageTag::parse("en")?];
    editor.playback.set_render_settings(settings)?;
    editor.select([first].into_iter().collect()).unwrap();
    Ok((editor, first, second))
}

pub(super) fn readme_boundary(
    app: &mut AppHarness,
    run: &Path,
    index: usize,
    label: &str,
) -> Result<serde_json::Value, Failure> {
    readme_seek(app, Tick(3 * SECOND))?;
    let raw = run.join(format!("boundary-{index}.png"));
    readme_capture(app, &raw)?;
    let editor = readme_editor(app)?;
    let project = editor.session.project();
    let snapshot = project::encode(project, false)?;
    let project_hash = sha256_bytes(&snapshot);
    let snapshot_name = format!("boundary-{index}.project");
    fs::write(run.join(&snapshot_name), &snapshot)?;
    let revision = serde_json::to_string(&editor.session.revision())?;
    let generation = editor.playback.generation();
    let frame_at = editor.playback.frame().map(|frame| frame.at.0);
    let presented = editor.presented_frame().map(|tick| tick.0);
    Ok(serde_json::json!({
        "boundary": index,
        "label": label,
        "raw": raw.file_name().map(|name| name.to_string_lossy().into_owned()),
        "snapshot": snapshot_name,
        "snapshot_sha256": project_hash,
        "revision": revision,
        "project_sha256": project_hash,
        "generation": generation,
        "frame_at": frame_at,
        "presented_at": presented
    }))
}

pub(super) fn verify_snapshots(
    run: &Path,
    first: ItemId,
    split: ItemId,
) -> Result<serde_json::Value, Failure> {
    let load = |index: usize| -> Result<project::ProjectData, Failure> {
        let bytes = fs::read(run.join(format!("boundary-{index}.project")))?;
        Ok(project::decode(&bytes)?.project.into_data())
    };
    let initial = load(0)?;
    let split_state = load(1)?;
    let trimmed_state = load(2)?;
    let undone_state = load(3)?;
    if undone_state != split_state {
        return Err("undo snapshot does not equal the split snapshot".into());
    }
    let split_right = split_state
        .scene()
        .timeline
        .main
        .items
        .iter()
        .find(|item| item.id == split)
        .ok_or("split snapshot has no right item")?
        .clone();
    let mut trimmed_reverted = trimmed_state.clone();
    {
        let right = trimmed_reverted
            .scene_mut()
            .timeline
            .main
            .items
            .iter_mut()
            .find(|item| item.id == split)
            .ok_or("trim snapshot has no right item")?;
        right.source_range = split_right.source_range;
        right.retime = split_right.retime;
    }
    if trimmed_reverted != split_state {
        return Err("trim snapshot changed more than the right item range/retime".into());
    }
    let initial_first = initial
        .scene()
        .timeline
        .main
        .items
        .iter()
        .find(|item| item.id == first)
        .ok_or("initial snapshot has no first item")?
        .clone();
    let mut split_reverted = split_state.clone();
    split_reverted
        .scene_mut()
        .timeline
        .main
        .items
        .retain(|item| item.id != split);
    {
        let left = split_reverted
            .scene_mut()
            .timeline
            .main
            .items
            .iter_mut()
            .find(|item| item.id == first)
            .ok_or("split snapshot has no first item")?;
        left.source_range = initial_first.source_range;
        left.retime = initial_first.retime;
    }
    if split_reverted != initial {
        return Err(
            "split snapshot changed more than the first item range/retime and new right item"
                .into(),
        );
    }
    Ok(serde_json::json!({
        "undo_matches_split": true,
        "trim_only_right_range": true,
        "split_only_first_range_and_new_item": true
    }))
}
