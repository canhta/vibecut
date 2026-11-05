//! Offscreen UI audit scenes for design review.
//!
//! `cargo xtask ui-audit` renders named scenes through the real [`Editor`] and
//! [`Workspace`] widget paths at 1x and 2x, plus an `index.html` that places each
//! render beside its matching `.spikes/ui-reference/` image when one exists.
//! This is an audit tool, not a test: it never runs in `cargo xtask ci`.
#[path = "../tests/it/support/shorts.rs"]
mod shorts_fixture;
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Command, CreatedId, NewItem, Placement, Session, TrackPlacement};
use media::{
    AssetData, AssetId, AudioInfo, ColorInfo, ContentHash, MediaError, SourceAsset, SourceProbe,
    SourceStream, StreamKind, TimeBase, VideoInfo,
};
use project::{
    AudioParams, Color, ItemId, ItemParams, OverlayParams, Project, ProjectId, TextParams, TrackId,
    TrackKind, VideoParams,
};
use render::{AudioBlock, ExecutionHooks, Image, MemoryKind, SourceProvider, UnavailableReason};
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor, Inspection,
    catalog::{Locale, Preferences},
    egui,
    import::ImportProbe,
    model_manager::ModelManager,
    playback::{Files, Playback, Sources},
    workspace::Workspace,
};

const SECOND: i64 = TICKS_PER_SECOND;
const WINDOW: [f32; 2] = [1440.0, 900.0];
const SCALES: [f32; 2] = [1.0, 2.0];

type Failure = Box<dyn std::error::Error>;
type AppHarness = Harness<'static, AuditApp>;

/// Audit roots: Home, the full editor and the dedicated Shorts window.
enum App {
    Home {
        workspace: Box<Workspace>,
        _directory: tempfile::TempDir,
    },
    Editor(Box<Editor>),
    Shorts(Box<vibecut::shorts::Shorts>),
}
impl App {
    fn ui(&mut self, ui: &mut egui::Ui) {
        match self {
            Self::Home { workspace, .. } => workspace.ui(ui),
            Self::Editor(editor) => editor.ui(ui),
            Self::Shorts(shorts) => shorts.ui(ui),
        }
    }
    fn busy(&self) -> bool {
        match self {
            Self::Home { workspace, .. } => workspace.busy(),
            Self::Editor(_) => false,
            Self::Shorts(shorts) => shorts.busy(),
        }
    }
    fn inspect(&self) -> Result<Inspection, Failure> {
        match self {
            Self::Editor(editor) => Ok(editor.inspect()?),
            Self::Home { .. } => Err("the Home scene has no editor inspection".into()),
            Self::Shorts(_) => Err("the Shorts scene has no editor inspection".into()),
        }
    }
    fn presented_frame(&self) -> Option<Tick> {
        match self {
            Self::Editor(editor) => editor.presented_frame(),
            Self::Shorts(shorts) => shorts.preview().and_then(|editor| editor.presented_frame()),
            Self::Home { workspace, .. } => workspace
                .editor
                .as_ref()
                .and_then(|editor| editor.presented_frame()),
        }
    }
}

/// eframe mode hands the app the same margin-free root Ui the native shell gets;
/// kittest's plain Ui mode inserts an 8 px central-panel frame that the real
/// window does not have.
struct AuditApp {
    app: App,
    _model_cache: Option<tempfile::TempDir>,
}
impl AuditApp {
    fn busy(&self) -> bool {
        self.app.busy()
    }
    fn inspect(&self) -> Result<Inspection, Failure> {
        self.app.inspect()
    }
    fn presented_frame(&self) -> Option<Tick> {
        self.app.presented_frame()
    }
    fn session(&self) -> Option<&Session> {
        match &self.app {
            App::Editor(editor) => Some(&editor.session),
            App::Home { .. } => None,
            App::Shorts(shorts) => shorts.preview().map(|editor| &*editor.session),
        }
    }
}
impl eframe::App for AuditApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Theme {
    Dark,
    Light,
}
impl Theme {
    const ALL: [Self; 2] = [Self::Dark, Self::Light];
    fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
    fn dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

struct Scene {
    name: &'static str,
    reference_dark: Option<&'static str>,
    reference_light: Option<&'static str>,
    build: fn(Theme, f32) -> Result<AppHarness, Failure>,
}
impl Scene {
    fn reference(&self, theme: Theme) -> Option<&'static str> {
        match theme {
            Theme::Dark => self.reference_dark,
            Theme::Light => self.reference_light,
        }
    }
}

const SCENES: &[Scene] = &[
    Scene {
        name: "timeline-zoom-low",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| timeline_zoom(theme, scale, 0.5),
    },
    Scene {
        name: "timeline-zoom-high",
        reference_dark: None,
        reference_light: None,
        build: |theme, scale| timeline_zoom(theme, scale, 100.0),
    },
    Scene {
        name: "shorts-pick",
        reference_dark: Some("screens-2026-11-04/shorts-1-pick.png"),
        reference_light: Some("screens-2026-11-04/shorts-1-pick.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::En, "pick"),
    },
    Scene {
        name: "shorts-setup",
        reference_dark: Some("screens-2026-11-04/shorts-2-setup.png"),
        reference_light: Some("screens-2026-11-04/shorts-2-setup.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::En, "setup"),
    },
    Scene {
        name: "shorts-review",
        reference_dark: Some("screens-2026-11-04/shorts-7-quick-edit-layout.png"),
        reference_light: Some("screens-2026-11-04/shorts-7-quick-edit-layout.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::En, "review"),
    },
    Scene {
        name: "shorts-pick-vi",
        reference_dark: Some("screens-2026-11-04/shorts-1-pick.png"),
        reference_light: Some("screens-2026-11-04/shorts-1-pick.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::Vi, "pick"),
    },
    Scene {
        name: "shorts-setup-vi",
        reference_dark: Some("screens-2026-11-04/shorts-2-setup.png"),
        reference_light: Some("screens-2026-11-04/shorts-2-setup.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::Vi, "setup"),
    },
    Scene {
        name: "shorts-review-vi",
        reference_dark: Some("screens-2026-11-04/shorts-7-quick-edit-layout.png"),
        reference_light: Some("screens-2026-11-04/shorts-7-quick-edit-layout.png"),
        build: |theme, scale| shorts_scene(theme, scale, Locale::Vi, "review"),
    },
    Scene {
        name: "models",
        reference_dark: None,
        reference_light: None,
        build: models,
    },
    Scene {
        name: "models-vi",
        reference_dark: None,
        reference_light: None,
        build: models_vi,
    },
    Scene {
        name: "home",
        reference_dark: Some("capcut/capcut-desktop-my-projects.png"),
        reference_light: Some("capcut/capcut-desktop-my-projects.png"),
        build: home,
    },
    Scene {
        name: "editor-empty",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: editor_empty,
    },
    Scene {
        name: "editor-media",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: editor_media,
    },
    Scene {
        name: "timeline-selection",
        reference_dark: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        build: timeline_selection,
    },
    Scene {
        name: "timeline-selection-vi",
        reference_dark: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        build: timeline_selection_vi,
    },
    Scene {
        name: "inspector-mixed-selection",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: inspector_mixed,
    },
    Scene {
        name: "inspector-mixed-selection-vi",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: inspector_mixed_vi,
    },
    Scene {
        name: "inspector-blend",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: inspector_blend,
    },
    Scene {
        name: "inspector-blend-vi",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: inspector_blend_vi,
    },
    Scene {
        name: "import-progress",
        reference_dark: Some("capcut/illustrations/capcut-modify-project.png"),
        reference_light: Some("capcut/illustrations/capcut-modify-project.png"),
        build: import_progress,
    },
    Scene {
        name: "import-complete-vi",
        reference_dark: Some("capcut/illustrations/capcut-modify-project.png"),
        reference_light: Some("capcut/illustrations/capcut-modify-project.png"),
        build: import_complete_vi,
    },
    Scene {
        name: "export-dialog",
        reference_dark: Some("capcut/illustrations/capcut-export-panel.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-export-settings.png"),
        build: export_dialog,
    },
    Scene {
        name: "close-prompt",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: close_prompt,
    },
    Scene {
        name: "close-prompt-vi",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui.png"),
        build: close_prompt_vi,
    },
    Scene {
        name: "tracks",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui-changes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui-changes.png"),
        build: tracks,
    },
    Scene {
        name: "scene-delete",
        reference_dark: Some("capcut/illustrations/capcut-desktop-ui-changes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-ui-changes.png"),
        build: scene_delete,
    },
    Scene {
        name: "placement",
        reference_dark: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        reference_light: Some("capcut/illustrations/capcut-desktop-editor-keyframes.png"),
        build: placement,
    },
];

fn preferences(theme: Theme, locale: Locale) -> Preferences {
    Preferences {
        locale,
        dark: theme.dark(),
        ..Preferences::default()
    }
}

fn harness(app: App, scale: f32) -> AppHarness {
    harness_sized(app, scale, WINDOW)
}

fn harness_sized(app: App, scale: f32, size: [f32; 2]) -> AppHarness {
    Harness::builder()
        .with_size(egui::vec2(size[0], size[1]))
        .with_pixels_per_point(scale)
        .build_eframe(|_| AuditApp {
            app,
            _model_cache: None,
        })
}

fn home(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let directory = tempfile::tempdir()?;
    let workspace = Workspace::new(
        directory.path().join("recovery"),
        preferences(theme, Locale::En),
        None,
    )?;
    let mut app = harness(
        App::Home {
            workspace: Box::new(workspace),
            _directory: directory,
        },
        scale,
    );
    // Discovery starts in the background; Home shows New/Open once it settles.
    for _ in 0..500 {
        app.run_steps(1);
        if !app.state().busy() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.run_steps(2);
    Ok(app)
}

fn empty_session(name: &str) -> Result<Session, Failure> {
    Ok(Session::new(
        Project::empty(ProjectId(46), name.into())?,
        50,
    )?)
}

fn app_editor(editor: Editor, scale: f32) -> AppHarness {
    harness(App::Editor(Box::new(editor)), scale)
}

fn empty_editor(theme: Theme, locale: Locale) -> Result<Editor, Failure> {
    let session = empty_session("Untitled Project")?;
    // Media import admits a 256 MiB probe/work lease before it opens a file.
    let mut playback = Playback::new(&session, ByteBudget::new(512 << 20)?, Arc::new(Files))?;
    playback.set_device_available(true);
    Ok(Editor::new(session, playback, preferences(theme, locale)))
}

fn editor_empty(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    Ok(app_editor(empty_editor(theme, Locale::En)?, scale))
}

struct NoTransfer;
impl ai::transport::DownloadTransport for NoTransfer {
    fn get(
        &self,
        _: &str,
        _: &[(String, String)],
    ) -> Result<ai::transport::HttpStreamResponse, ai::Error> {
        Err(ai::Error::Network)
    }
}

fn models(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    models_locale(theme, scale, Locale::En)
}

fn models_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    models_locale(theme, scale, Locale::Vi)
}

fn models_locale(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
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

fn source_stream(kind: StreamKind, duration: Tick) -> Result<SourceStream, Failure> {
    Ok(SourceStream {
        index: 0,
        kind,
        codec: match kind {
            StreamKind::Video => "h264".into(),
            StreamKind::Audio => "aac".into(),
            StreamKind::Other => "data".into(),
        },
        time_base: TimeBase::new(1, 1_000)?,
        duration: Some(duration),
        start: None,
        frame_rate: (kind == StreamKind::Video).then_some(FrameRate::FPS_30),
        frame_count: None,
        variable_frame_rate: None,
        video: (kind == StreamKind::Video).then_some(VideoInfo {
            width: 1920,
            height: 1080,
            color: ColorInfo::default(),
            bit_depth: Some(8),
        }),
        audio: (kind == StreamKind::Audio).then_some(AudioInfo {
            sample_rate: 48_000,
            channels: 2,
            initial_padding: 0,
            trailing_padding: 0,
        }),
    })
}

fn source_asset(id: u64, kind: StreamKind, path: PathBuf) -> Result<SourceAsset, Failure> {
    let duration = Tick(10 * SECOND);
    Ok(SourceAsset::try_from(AssetData {
        ephemeral: false,
        id: AssetId(id),
        path: std::path::absolute(path)?,
        content_hash: ContentHash::from_bytes(&[id as u8; 8]),
        probe: SourceProbe {
            format: "mov,mp4".into(),
            duration: Some(duration),
            streams: vec![source_stream(kind, duration)?],
        },
    })?)
}

fn range(start: i64, end: i64) -> Result<SourceRange, Failure> {
    Ok(SourceRange::new(Tick(start * SECOND), Tick(end * SECOND))?)
}

fn track(session: &mut Session, name: &str, kind: TrackKind) -> Result<TrackId, Failure> {
    let outcome = session.apply(Command::AddTrack {
        placement: TrackPlacement::Auto,
        name: name.into(),
        kind,
    })?;
    Ok(outcome
        .created
        .into_iter()
        .find_map(|created| match created {
            CreatedId::Track(id) => Some(id),
            _ => None,
        })
        .ok_or("created track id")?)
}

fn insert(
    session: &mut Session,
    track: TrackId,
    start: Tick,
    source_range: SourceRange,
    asset: Option<AssetId>,
    content: ItemParams,
) -> Result<ItemId, Failure> {
    let outcome = session.apply(Command::InsertItem {
        placement: Placement::Track { track },
        item: NewItem {
            animations: Default::default(),
            source_asset: asset,
            source_range,
            start,
            content,
        },
    })?;
    Ok(outcome
        .created
        .into_iter()
        .find_map(|created| match created {
            CreatedId::Item(id) => Some(id),
            _ => None,
        })
        .ok_or("created item id")?)
}

/// Deterministic healthy media for populated-editor scenes. The provider is
/// injected through the same public `Sources` seam production `Files` uses, so
/// the audit exercises the real preview/render path without opening a file.
struct SyntheticSources;
impl Sources for SyntheticSources {
    fn create(&self) -> Box<dyn SourceProvider> {
        Box::new(SyntheticFrames)
    }
}
struct SyntheticFrames;
impl SourceProvider for SyntheticFrames {
    fn status(
        &mut self,
        _: &SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> render::Result<Option<UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: ItemId,
        _: Tick,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<Option<Image>> {
        const WIDTH: u32 = 640;
        const HEIGHT: u32 = 360;
        let mut bytes = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let [red, green, blue] = if (160..480).contains(&x) && (90..270).contains(&y) {
                    [238, 196, 52]
                } else {
                    [
                        (24 + x * 180 / WIDTH) as u8,
                        (58 + y * 130 / HEIGHT) as u8,
                        148,
                    ]
                };
                bytes.extend_from_slice(&[red, green, blue, 255]);
            }
        }
        Ok(Some(Image::from_rgba8(
            WIDTH, HEIGHT, &bytes, false, 13, hooks,
        )?))
    }
    fn audio(
        &mut self,
        _: &SourceAsset,
        _: ItemId,
        _: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<AudioBlock> {
        AudioBlock::new(rate, 2, frames, MemoryKind::Decoded, hooks)
    }
}

fn wait_for_frame(editor: &mut Editor) -> Result<(), Failure> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while editor.playback.frame().is_none() {
        editor.playback.poll()?;
        if Instant::now() >= deadline {
            return Err("preview frame timeout".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

fn wait_for_presented(app: &mut AppHarness) -> Result<(), Failure> {
    for _ in 0..500 {
        app.run_steps(1);
        if app.state().presented_frame().is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Err("preview frame was not presented".into())
}

fn media_editor(theme: Theme, locale: Locale) -> Result<(Editor, ItemId), Failure> {
    let mut session = empty_session("Launch cut")?;
    for (id, kind) in [(1, StreamKind::Video), (2, StreamKind::Audio)] {
        session.apply(Command::AddAsset {
            asset: source_asset(id, kind, PathBuf::from(format!("/media/source-{id}.mp4")))?,
        })?;
    }
    let main = session.project().data().scene().timeline.main.id;
    let first = insert(
        &mut session,
        main,
        Tick::ZERO,
        range(0, 6)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
        }),
    )?;
    insert(
        &mut session,
        main,
        Tick(6 * SECOND),
        range(6, 10)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
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
            gain_db: 0.0,
            muted: false,
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
        }),
    )?;
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20)?,
        Arc::new(SyntheticSources),
    )?;
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, preferences(theme, locale));
    let project = editor.session.project().clone();
    editor.playback.seek(Tick(3 * SECOND), false, project)?;
    wait_for_frame(&mut editor)?;
    Ok((editor, first))
}

fn timeline_zoom(theme: Theme, scale: f32, zoom: f64) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    editor.session.execute(
        "project.set_view_state",
        serde_json::json!({"zoom":zoom,"scroll":0,"playhead":0}),
    )?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

fn editor_media(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (editor, _) = media_editor(theme, Locale::En)?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(2);
    Ok(app)
}

fn timeline_selection_locale(
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

fn timeline_selection(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    timeline_selection_locale(theme, scale, Locale::En)
}

fn timeline_selection_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    timeline_selection_locale(theme, scale, Locale::Vi)
}

fn widget_rect(app: &AppHarness, id: &str) -> Result<egui::Rect, Failure> {
    app.state()
        .inspect()?
        .widgets
        .into_iter()
        .find(|widget| widget.id == id)
        .map(|widget| widget.rect)
        .ok_or_else(|| format!("missing widget {id}").into())
}

fn inspector_mixed_locale(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
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
    editor.selection = [first, second, background].into_iter().collect();
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

fn inspector_mixed(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_mixed_locale(theme, scale, Locale::En)
}

fn inspector_mixed_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_mixed_locale(theme, scale, Locale::Vi)
}

fn inspector_blend_locale(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
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
    session.execute(
        "timeline.set_item_param",
        serde_json::json!({"item":item,"patch":{"field":"blendMode","value":"multiply"}}),
    )?;
    session.execute(
        "timeline.set_item_param",
        serde_json::json!({"item":item,"patch":{"field":"opacity","value":0.5}}),
    )?;
    let mut playback = Playback::new(&session, ByteBudget::new(64 << 20)?, Arc::new(Files))?;
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, preferences(theme, locale));
    editor.selection.insert(item);
    wait_for_frame(&mut editor)?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(3);
    Ok(app)
}

fn inspector_blend(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_blend_locale(theme, scale, Locale::En)
}

fn inspector_blend_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    inspector_blend_locale(theme, scale, Locale::Vi)
}

/// Returns a deterministic library asset for every path except `bad`, which
/// exercises the real partial-import failure state.
struct CompletedProbe;
impl ImportProbe for CompletedProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        _: Arc<std::sync::atomic::AtomicBool>,
    ) -> media::Result<SourceAsset> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name.contains("bad") {
            return Err(MediaError::Unsupported("audit import refusal"));
        }
        let kind = if name.contains("audio") {
            StreamKind::Audio
        } else {
            StreamKind::Video
        };
        source_asset(id.0, kind, path).map_err(|_| MediaError::Unsupported("fixture"))
    }
}

fn import_complete(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(theme, locale)?;
    editor.set_import_probe(Arc::new(CompletedProbe))?;
    editor.drop_files(vec![
        PathBuf::from("/media/video-a.mp4"),
        PathBuf::from("/media/bad-clip.mp4"),
        PathBuf::from("/media/audio-a.wav"),
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

fn import_complete_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    import_complete(theme, scale, Locale::Vi)
}

fn close_prompt_locale(theme: Theme, scale: f32, locale: Locale) -> Result<AppHarness, Failure> {
    let (editor, _) = media_editor(theme, locale)?;
    let directory = tempfile::tempdir()?;
    let mut workspace = Workspace::new(
        directory.path().join("recovery"),
        preferences(theme, locale),
        None,
    )?;
    workspace.editor = Some(Box::new(editor));
    workspace.request_close();
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

fn close_prompt(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    close_prompt_locale(theme, scale, Locale::En)
}

fn close_prompt_vi(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    close_prompt_locale(theme, scale, Locale::Vi)
}

const TRACKS_WINDOW: [f32; 2] = [1728.0, 1200.0];

fn tracks(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
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

fn scene_delete(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Scenes")?;
    let mut ids = Vec::new();
    for name in ["Opening", "Closing"] {
        let created = session.execute("scene.create", serde_json::json!({"name": name}))?;
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

fn placement(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut session = empty_session("Placement")?;
    for (id, kind) in [(1, StreamKind::Video), (2, StreamKind::Audio)] {
        session.apply(Command::AddAsset {
            asset: source_asset(id, kind, PathBuf::from(format!("/media/source-{id}.mp4")))?,
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
            source_audio_enabled: true,
        }),
    )?;
    insert(
        &mut session,
        video,
        Tick(6 * SECOND),
        range(6, 10)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
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
            gain_db: 0.0,
            muted: false,
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

/// Blocks every file after the first until the batch is cancelled, so the
/// audit samples the real progress overlay while the job is genuinely running.
struct ProgressProbe {
    calls: AtomicUsize,
    release: (Mutex<bool>, Condvar),
}
impl ProgressProbe {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            release: (Mutex::new(false), Condvar::new()),
        }
    }
}
impl ImportProbe for ProgressProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> media::Result<SourceAsset> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            let mut released = match self.release.0.lock() {
                Ok(released) => released,
                Err(_) => return Err(MediaError::Cancelled),
            };
            while !*released {
                if cancelled.load(Ordering::Acquire) {
                    return Err(MediaError::Cancelled);
                }
                released = match self
                    .release
                    .1
                    .wait_timeout(released, Duration::from_millis(5))
                {
                    Ok((released, _)) => released,
                    Err(_) => return Err(MediaError::Cancelled),
                };
            }
        }
        source_asset(id.0, StreamKind::Video, path).map_err(|_| MediaError::Unsupported("fixture"))
    }
}

fn export_dialog(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let (mut editor, _) = media_editor(theme, Locale::En)?;
    editor.activate("file.export")?;
    let mut app = app_editor(editor, scale);
    wait_for_presented(&mut app)?;
    app.run_steps(4);
    Ok(app)
}

fn import_progress(theme: Theme, scale: f32) -> Result<AppHarness, Failure> {
    let mut editor = empty_editor(theme, Locale::En)?;
    editor.set_import_probe(Arc::new(ProgressProbe::new()))?;
    editor.drop_files(vec![
        PathBuf::from("/media/one.mp4"),
        PathBuf::from("/media/two.mp4"),
        PathBuf::from("/media/three.mp4"),
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

fn main() -> Result<(), Failure> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize()?;
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        Some(path) => root.join(path),
        None => root.join("target"),
    };
    let output = target.join("ui-audit");
    std::fs::create_dir_all(&output)?;
    let references = reference_root(&root);
    let selected = select(std::env::args().skip(1))?;
    let mut rows = Vec::new();
    for (scene, theme) in &selected {
        let variant = format!("{}-{}", scene.name, theme.key());
        let mut images = Vec::new();
        for scale in SCALES {
            let mut app = (scene.build)(*theme, scale)?;
            app.run_steps(2);
            let image = app.render().map_err(std::io::Error::other)?;
            let image = opaque(image, vibecut::window_color(theme.dark()));
            let file = format!("{variant}@{}x.png", scale as u32);
            image.save(output.join(&file))?;
            images.push((file, scale));
        }
        let reference = scene.reference(*theme).and_then(|relative| {
            copy_reference(references.as_deref(), &output, &variant, relative)
        });
        rows.push(Row {
            variant,
            theme: theme.key(),
            images,
            reference,
        });
        eprintln!("ui-audit: {} rendered", scene.name);
    }
    let index = output.join("index.html");
    std::fs::write(&index, index_html(&rows))?;
    println!("ui-audit: {}", index.display());
    Ok(())
}

fn select(names: impl Iterator<Item = String>) -> Result<Vec<(&'static Scene, Theme)>, Failure> {
    let names: Vec<String> = names.collect();
    if names.is_empty() {
        return Ok(SCENES
            .iter()
            .flat_map(|scene| Theme::ALL.map(|theme| (scene, theme)))
            .collect());
    }
    let mut selected = BTreeSet::new();
    let mut unknown = Vec::new();
    for name in names {
        let mut matched = false;
        for (index, scene) in SCENES.iter().enumerate() {
            if name == scene.name {
                for (theme_index, _theme) in Theme::ALL.into_iter().enumerate() {
                    selected.insert((index, theme_index));
                }
                matched = true;
            } else {
                for (theme_index, theme) in Theme::ALL.into_iter().enumerate() {
                    if name == format!("{}-{}", scene.name, theme.key()) {
                        selected.insert((index, theme_index));
                        matched = true;
                    }
                }
            }
        }
        if !matched {
            unknown.push(name);
        }
    }
    if !unknown.is_empty() {
        if unknown.iter().any(|name| name == "settings") {
            return Err(
                "scene `settings` is unavailable: main exposes language/theme in the editor header; the settings dialog and its audit scene belong to p1-parity/75"
                    .into(),
            );
        }
        let known: Vec<_> = SCENES
            .iter()
            .flat_map(|scene| {
                Theme::ALL
                    .into_iter()
                    .map(move |theme| format!("{}-{}", scene.name, theme.key()))
            })
            .collect();
        return Err(format!(
            "unknown scene(s): {}; known scenes: {}",
            unknown.join(", "),
            known.join(", ")
        )
        .into());
    }
    selected
        .into_iter()
        .map(|(index, theme_index)| {
            let scene = SCENES.get(index).ok_or("scene index")?;
            let theme = Theme::ALL
                .into_iter()
                .nth(theme_index)
                .ok_or("theme index")?;
            Ok((scene, theme))
        })
        .collect()
}

fn reference_root(root: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("VIBECUT_UI_REFERENCE") {
        let path = PathBuf::from(path);
        return path.is_dir().then_some(path);
    }
    let local = root.join(".spikes/ui-reference");
    if local.is_dir() {
        return Some(local);
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let listing = String::from_utf8(output.stdout).ok()?;
    listing
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .map(|worktree| worktree.join(".spikes/ui-reference"))
        .filter(|path| path.is_dir())
}

fn copy_reference(
    root: Option<&Path>,
    output: &Path,
    variant: &str,
    relative: &str,
) -> Option<String> {
    let source = root?.join(relative);
    let extension = source.extension()?.to_str()?;
    let name = format!("{variant}.{extension}");
    let directory = output.join("reference");
    std::fs::create_dir_all(&directory).ok()?;
    std::fs::copy(&source, directory.join(&name)).ok()?;
    Some(format!("reference/{name}"))
}

/// egui renders premultiplied alpha over a transparent clear; the audit PNG is
/// opaque so it can be compared and reviewed outside a compositor.
fn opaque(image: image::RgbaImage, background: egui::Color32) -> image::RgbaImage {
    let [red, green, blue, _] = background.to_array();
    let mut output = image::RgbaImage::new(image.width(), image.height());
    for (x, y, pixel) in image.enumerate_pixels() {
        let [source_red, source_green, source_blue, source_alpha] = pixel.0;
        let inverse = 255 - u32::from(source_alpha);
        let blend = |source: u8, destination: u8| {
            (u32::from(source) + u32::from(destination) * inverse / 255).min(255) as u8
        };
        output.put_pixel(
            x,
            y,
            image::Rgba([
                blend(source_red, red),
                blend(source_green, green),
                blend(source_blue, blue),
                255,
            ]),
        );
    }
    output
}

struct Row {
    variant: String,
    theme: &'static str,
    images: Vec<(String, f32)>,
    reference: Option<String>,
}

fn index_html(rows: &[Row]) -> String {
    let mut html = String::from(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>VibeCut UI audit</title>\n<style>\n\
         body{font:14px/1.4 system-ui,sans-serif;margin:24px;background:#0f1113;color:#fff}\n\
         h1{font-size:24px} h2{font-size:18px;margin-top:32px}\n\
         table{border-collapse:collapse;width:100%}\n\
         th,td{text-align:left;vertical-align:top;padding:8px;border-top:1px solid #26272a}\n\
         img{max-width:100%;display:block;border:1px solid #26272a}\n\
         .missing{color:#898a8b;font-style:italic}\n\
         </style>\n</head>\n<body>\n<h1>VibeCut UI audit</h1>\n\
         <p>Offscreen renders at 1x and 2x. Reference images are copied from \
         <code>.spikes/ui-reference/</code> when present.</p>\n",
    );
    let mut current = "";
    for Row {
        variant,
        theme,
        images,
        reference,
    } in rows
    {
        let scene = variant
            .strip_suffix("-dark")
            .or_else(|| variant.strip_suffix("-light"))
            .unwrap_or(variant);
        if scene != current {
            if !current.is_empty() {
                let _ = writeln!(html, "</table>");
            }
            let _ = writeln!(html, "<h2>{scene}</h2>\n<table>");
            let _ = writeln!(
                html,
                "<tr><th>theme</th><th>1x</th><th>2x</th><th>reference</th></tr>"
            );
            current = scene;
        }
        let _ = writeln!(html, "<tr><th>{theme}</th>");
        for (file, _) in images {
            let _ = writeln!(
                html,
                "<td><a href=\"{file}\"><img src=\"{file}\" alt=\"{variant}\"></a></td>"
            );
        }
        match reference {
            Some(reference) => {
                let _ = writeln!(
                    html,
                    "<td><img src=\"{reference}\" alt=\"reference for {variant}\"></td></tr>"
                );
            }
            None => {
                let _ = writeln!(
                    html,
                    "<td class=\"missing\">no matching reference found</td></tr>"
                );
            }
        }
    }
    if !rows.is_empty() {
        html.push_str("</table>\n");
    }
    html.push_str("</body>\n</html>\n");
    html
}

/// Shared fixture, real widget path; no provider calls or visible native window.
fn shorts_scene(
    theme: Theme,
    scale: f32,
    locale: Locale,
    scene: &str,
) -> Result<AppHarness, Failure> {
    use vibecut::{
        playback::Sources,
        shorts::{Decision, Shorts},
    };
    static ASSET: std::sync::OnceLock<SourceAsset> = std::sync::OnceLock::new();
    let mut source = shorts_fixture::source();
    let preview: Arc<dyn Sources> = if let Some(path) = std::env::var_os("SHORTS_AUDIT_VIDEO") {
        let asset = match ASSET.get() {
            Some(asset) => asset.clone(),
            None => {
                let asset = SourceAsset::import(AssetId(1), PathBuf::from(path))?;
                let _ = ASSET.set(asset.clone());
                asset
            }
        };
        let mut data = source.project.into_data();
        data.assets = vec![asset];
        source.project = Project::new(data)?;
        Arc::new(Files)
    } else {
        Arc::new(shorts_fixture::Pixels)
    };
    let prefs = preferences(theme, locale);
    let mut shorts = Shorts::with_generator(
        prefs,
        ByteBudget::new(768 << 20)?,
        Arc::new(shorts_fixture::Once(Mutex::new(Some(source)))),
        preview,
    )?;
    shorts.captions = false;
    if scene != "pick" {
        shorts.pick_video(std::env::temp_dir().join("Interview.mp4"))?;
    }
    let mut app = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_pixels_per_point(scale)
        .build_eframe(|_| AuditApp {
            app: App::Shorts(Box::new(shorts)),
            _model_cache: None,
        });
    app.run_steps(3);
    if scene == "review" {
        app.get(By::new().predicate(|n| n.author_id() == Some("shorts.generate")))
            .click();
        app.run_steps(1);
        let App::Shorts(shorts) = &mut app.state_mut().app else {
            return Err("missing Shorts audit app".into());
        };
        shorts_fixture::wait(shorts);
        app.run_steps(3);
        app.get(By::new().predicate(|n| n.author_id() == Some("shorts.accept")))
            .click();
        app.run_steps(2);
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let App::Shorts(shorts) = &mut app.state_mut().app else {
                return Err("missing Shorts audit app".into());
            };
            shorts.tick();
            if let Some(error) = shorts.error() {
                return Err(std::io::Error::other(error.to_string()).into());
            }
            if let Some(error) = shorts.preview().and_then(|p| p.playback.error()) {
                return Err(std::io::Error::other(error).into());
            }
            if shorts
                .preview()
                .is_some_and(|p| p.playback.frame().is_some())
            {
                if shorts
                    .reviews()
                    .first()
                    .is_none_or(|r| r.decision != Decision::Accepted)
                {
                    return Err("Shorts review action did not apply".into());
                }
                break;
            }
            if std::time::Instant::now() > deadline {
                return Err("Shorts preview timeout".into());
            }
            std::thread::yield_now();
        }
        app.run_steps(3);
    }
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_selection_preview_matches_its_edited_timeline() -> Result<(), Failure> {
        let app = inspector_mixed_vi(Theme::Light, 1.0)?;
        let App::Editor(editor) = &app.state().app else {
            return Err("expected editor scene".into());
        };
        let frame = editor.playback.frame().ok_or("missing preview frame")?;
        // This corner lies outside the text. It must show the scene's solid
        // overlay, rather than the synthetic video deleted during setup.
        let corner = frame.image.pixels.first().ok_or("empty preview frame")?;
        assert_eq!(*corner, egui::Color32::from_rgb(35, 45, 65));
        Ok(())
    }
}
