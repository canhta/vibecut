//! Provides shared deterministic media fixtures, session builders, sources, probes.
use super::Failure;
use super::application::{App, AppHarness, SECOND, Theme, harness, preferences};
use engine::ByteBudget;
use engine::Command;
use engine::CreatedId;
use engine::NewItem;
use engine::Placement;
use engine::Session;
use engine::TrackPlacement;
use media::AssetId;
use media::ContentHash;
use media::MediaError;
use media::SourceAsset;
use media::SourceStream;
use media::StreamKind;
use project::AudioParams;
use project::Color;
use project::ItemId;
use project::ItemParams;
use project::Project;
use project::ProjectId;
use project::TextParams;
use project::TrackId;
use project::TrackKind;
use project::VideoParams;
use render::AudioBlock;
use render::ExecutionHooks;
use render::Image;
use render::MemoryKind;
use render::SourceProvider;
use render::UnavailableReason;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use time::SourceRange;
use time::Tick;
use vibecut::Editor;
use vibecut::ImportProbe;
use vibecut::egui;
use vibecut::i18n::Locale;
use vibecut::services::playback::Files;
use vibecut::services::playback::Playback;
use vibecut::services::playback::Sources;

pub(super) fn empty_session(name: &str) -> Result<Session, Failure> {
    Ok(Session::new(
        Project::empty(ProjectId(46), name.into())?,
        50,
    )?)
}

pub(super) fn app_editor(editor: Editor, scale: f32) -> AppHarness {
    harness(App::Editor(Box::new(editor)), scale)
}

pub(super) fn empty_editor(theme: Theme, locale: Locale) -> Result<Editor, Failure> {
    let session = empty_session("Untitled Project")?;
    // Media import admits a 256 MiB probe/work lease before it opens a file.
    let mut playback = Playback::new(&session, ByteBudget::new(512 << 20)?, Arc::new(Files))?;
    playback.set_device_available(true);
    Ok(Editor::new(session, playback, preferences(theme, locale)))
}
pub(super) struct NoTransfer;
impl transport::DownloadTransport for NoTransfer {
    fn get(
        &self,
        _: &str,
        _: &[(String, String)],
    ) -> Result<transport::HttpStreamResponse, transport::Error> {
        Err(transport::Error::Network)
    }
}
pub(super) fn source_stream(kind: StreamKind, duration: Tick) -> Result<SourceStream, Failure> {
    Ok(testkit::source::stream(kind).duration(duration).build())
}

pub(super) fn source_asset(
    id: u64,
    kind: StreamKind,
    path: PathBuf,
) -> Result<SourceAsset, Failure> {
    let path = std::path::absolute(path)?;
    let duration = Tick(10 * SECOND);
    Ok(testkit::source::asset(id, path)
        .format("mov,mp4")
        .duration(duration)
        .content_hash(ContentHash::from_bytes(&[id as u8; 8]))
        .stream(source_stream(kind, duration)?)
        .build())
}

pub(super) fn range(start: i64, end: i64) -> Result<SourceRange, Failure> {
    Ok(SourceRange::new(Tick(start * SECOND), Tick(end * SECOND))?)
}

pub(super) fn track(
    session: &mut Session,
    name: &str,
    kind: TrackKind,
) -> Result<TrackId, Failure> {
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

pub(super) fn insert(
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
            retime: Default::default(),
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
pub(super) struct SyntheticSources;
impl Sources for SyntheticSources {
    fn create(&self) -> Box<dyn SourceProvider> {
        Box::new(SyntheticFrames)
    }
}
pub(super) struct SyntheticFrames;
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

pub(super) fn wait_for_frame(editor: &mut Editor) -> Result<(), Failure> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while editor.playback.frame().is_none() {
        editor.playback.poll(std::time::Instant::now())?;
        if Instant::now() >= deadline {
            return Err("preview frame timeout".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

pub(super) fn wait_for_presented(app: &mut AppHarness) -> Result<(), Failure> {
    for _ in 0..500 {
        app.run_steps(1);
        if app.state().presented_frame().is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Err("preview frame was not presented".into())
}

/// Capture settled pixels: wait for every async effect preview, then load the
/// finished textures and paint the frame the audit reads.
pub(super) fn settle(app: &mut AppHarness) -> Result<(), Failure> {
    let timeout = vibecut::SETTLE_BUDGET;
    match &mut app.state_mut().app {
        App::Editor(editor) | App::Player(editor) | App::Assets(editor) => {
            editor.settle_previews(timeout)?;
        }
        App::Home { workspace, .. } => {
            if let Some(editor) = workspace.focused_editor_mut() {
                editor.settle_previews(timeout)?;
            }
        }
        App::Shorts(_) => {}
    }
    app.run_steps(2);
    Ok(())
}

pub(super) fn media_editor(theme: Theme, locale: Locale) -> Result<(Editor, ItemId), Failure> {
    let mut session = empty_session("Launch cut")?;
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
    let first = insert(
        &mut session,
        main,
        Tick::ZERO,
        range(0, 6)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
            cleanup: Default::default(),
            background: ClipBackground::None,
        }),
    )?;
    insert(
        &mut session,
        main,
        Tick(6 * SECOND),
        range(6, 10)?,
        Some(AssetId(1)),
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            crop: None,
            muted: false,
            cleanup: Default::default(),
            background: ClipBackground::None,
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
    let mut editor = Editor::new(session, playback, preferences(theme, locale));
    let project = editor.session.project().clone();
    editor.playback.seek(Tick(3 * SECOND), false, project)?;
    wait_for_frame(&mut editor)?;
    Ok((editor, first))
}

/// Keep the loading scene genuinely pending until its preview jobs are cancelled.
pub(super) struct LoadingSources;
impl Sources for LoadingSources {
    fn create(&self) -> Box<dyn SourceProvider> {
        Box::new(LoadingFrames)
    }
}
pub(super) struct LoadingFrames;
impl SourceProvider for LoadingFrames {
    fn status(
        &mut self,
        _: &SourceAsset,
        _: bool,
        hooks: &dyn ExecutionHooks,
    ) -> render::Result<Option<UnavailableReason>> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !hooks.cancelled() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        Err(render::Error::Cancelled)
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: ItemId,
        _: Tick,
        _: &dyn ExecutionHooks,
    ) -> render::Result<Option<Image>> {
        Ok(None)
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
pub(super) fn widget_rect(app: &AppHarness, id: &str) -> Result<egui::Rect, Failure> {
    app.state()
        .inspect()?
        .widgets
        .into_iter()
        .find(|widget| widget.id == id)
        .map(|widget| widget.rect)
        .ok_or_else(|| format!("missing widget {id}").into())
}
pub(super) fn import_summary(asset: SourceAsset) -> media::ImportedSource {
    let streams: Vec<_> = asset
        .probe()
        .streams
        .iter()
        .filter(|stream| stream.kind != StreamKind::Other)
        .map(|stream| (stream.index, stream.kind, stream.codec.clone()))
        .collect();
    media::ImportedSource {
        asset,
        capabilities: media::ProbeCapabilities::decodable(streams),
    }
}
/// Returns a deterministic library asset for every path except `bad`, which
/// exercises the real partial-import failure state.
pub(super) struct CompletedProbe;
impl ImportProbe for CompletedProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        _: Arc<std::sync::atomic::AtomicBool>,
    ) -> media::Result<media::ImportedSource> {
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
        source_asset(id.0, kind, path)
            .map(import_summary)
            .map_err(|_| MediaError::Unsupported("fixture"))
    }
}
/// Blocks every file after the first until the batch is cancelled, so the
/// audit samples the real progress overlay while the job is genuinely running.
pub(super) struct ProgressProbe {
    calls: AtomicUsize,
    release: (Mutex<bool>, Condvar),
}
impl ProgressProbe {
    pub(super) fn new() -> Self {
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
    ) -> media::Result<media::ImportedSource> {
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
        source_asset(id.0, StreamKind::Video, path)
            .map(import_summary)
            .map_err(|_| MediaError::Unsupported("fixture"))
    }
}
