//! Shorts state: explicit generation, undoable review and engine exports.
use crate::{
    Editor, Error, Result,
    catalog::{self, Locale, Preferences, Text},
    crop, egui,
    playback::{Files, Playback, Sources},
};
pub use engine::ClipCandidate as Candidate;
use engine::ClipSourceSegment;
use engine::{
    ByteBudget, ClipBatch, ClipBatchRequest, ClipShort, Command, NewItem, Session,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    clip_boundaries::{ClipBoundarySettings, snap_clip_boundaries},
    jobs::{
        CancelToken, JobId, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig,
        SystemClock,
    },
    subtitles::Cancellation,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
/// Generation's retained source context: trimming uses the same safe-edge owner
/// as generation, and captions remain in source coordinates until batch preparation.
pub struct ReviewSource {
    pub project: project::Project,
    pub asset: media::AssetId,
    pub transcript: captions::Transcript,
    pub cues: captions::CueTrack,
    pub energy: Vec<media::SpeechEnergyWindow>,
    pub boundaries: ClipBoundarySettings,
    pub candidates: Vec<Candidate>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Pending,
    Accepted,
    Rejected,
}
#[derive(Clone, Debug)]
pub struct Review {
    pub segments: Vec<ClipSourceSegment>,
    pub crop: render::Rect,
    pub decision: Decision,
}
impl PartialEq for Review {
    fn eq(&self, other: &Self) -> bool {
        self.crop == other.crop
            && self.decision == other.decision
            && self.segments.len() == other.segments.len()
            && self.segments.iter().zip(&other.segments).all(|(a, b)| {
                a.range == b.range
                    && a.role == b.role
                    && a.source_order == b.source_order
                    && a.repair.as_ref().map(std::mem::discriminant)
                        == b.repair.as_ref().map(std::mem::discriminant)
            })
    }
}
pub(super) struct Edit {
    pub(super) index: usize,
    pub(super) before: Review,
    pub(super) after: Review,
}
/// Explicit settings for one run. Provider/model ids come from capability data.
#[derive(Clone)]
pub struct GenerationRequest {
    pub path: PathBuf,
    pub language: String,
    pub transcription_provider: String,
    pub transcription_model: String,
    pub suggestion_provider: String,
    pub suggestion_model: String,
    pub count: usize,
    pub min_seconds: u32,
    pub max_seconds: u32,
    pub max_segments: usize,
}
/// Injectable composition boundary. Production calls the shared engine clip workflow;
/// tests inject bounded results/failures while using the same job and widget paths.
pub trait Generator: Send + Sync + 'static {
    fn run(
        &self,
        request: GenerationRequest,
        budget: ByteBudget,
        events: &TaskEvents,
        cancel: &Cancellation,
    ) -> engine::Result<ReviewSource>;
}
pub use crate::ai_task::TaskEvents;
pub(super) struct Generating {
    pub(super) id: JobId,
    pub(super) runner_cancel: CancelToken,
    pub(super) cancel: Cancellation,
    pub(super) events: Arc<TaskEvents>,
    pub(super) receive: mpsc::Receiver<Result<ReviewSource>>,
}
#[derive(Clone, Copy)]
pub(super) enum Pick {
    Video,
    Export,
}
pub(super) struct Picker {
    pub(super) future:
        std::pin::Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>>>>,
    pub(super) kind: Pick,
    pub(super) waker: std::task::Waker,
}
pub(super) struct Repaint(egui::Context);
impl std::task::Wake for Repaint {
    fn wake(self: Arc<Self>) {
        self.0.request_repaint();
    }
}

pub struct State {
    pub preferences: Preferences,
    pub request: GenerationRequest,
    pub(super) source: Option<ReviewSource>,
    pub(super) reviews: Vec<Review>,
    pub(super) selected: usize,
    pub(super) undo: VecDeque<Edit>,
    pub(super) redo: Vec<Edit>,
    pub(super) crop_entry: Option<Review>,
    pub(super) crop_cancelled: bool,
    pub(super) crop_focused: bool,
    pub(super) trim_drafts: Vec<(String, String)>,
    pub(super) draft_locale: Option<Locale>,
    pub(super) preview: Option<Editor>,
    pub(super) preview_dirty: bool,
    pub(super) preview_index: Option<usize>,
    pub(super) preview_sources: Arc<dyn Sources>,
    pub(super) generator: Arc<dyn Generator>,
    pub(super) generating: Option<Generating>,
    // Cancellation in Drop runs before runner joins an approval-waiting worker.
    pub(super) runner: JobRunner,
    pub(super) batch: Option<ClipBatch>,
    pub(super) budget: ByteBudget,
    pub(super) picker: Option<Picker>,
    pub(super) error: Option<Error>,
    pub(super) error_text: Text,
    pub(super) cancelled: bool,
    pub captions: bool,
    pub(super) tool: Option<super::view::Tool>,
    pub(super) list_open: bool,
    pub(super) options_open: bool,
    pub(super) frame: Option<super::view::FrameTexture>,
    /// False shows the grid of results; true the quick edit of the selected short.
    pub(super) editing: bool,
    pub(super) thumbs: Thumbs,
    pub(super) configured: bool,
}

/// Poster frames of the suggestions, decoded off the UI thread, then uploaded once.
#[derive(Default)]
pub(super) struct Thumbs {
    receive: Option<mpsc::Receiver<(usize, media::Thumbnail)>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    pub(super) images: Vec<Option<egui::ColorImage>>,
    pub(super) textures: Vec<Option<egui::TextureHandle>>,
}
impl Thumbs {
    pub(super) fn stop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
        self.receive = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.cancel = Arc::default();
        self.images.clear();
        self.textures.clear();
    }
    fn start(&mut self, asset: media::SourceAsset, starts: Vec<Tick>) {
        self.stop();
        let (send, receive) = mpsc::channel();
        let cancel = self.cancel.clone();
        self.images = starts.iter().map(|_| None).collect();
        self.textures = starts.iter().map(|_| None).collect();
        self.receive = Some(receive);
        self.worker = std::thread::Builder::new()
            .name("shorts-posters".into())
            .spawn(move || {
                for (index, at) in starts.into_iter().enumerate() {
                    if cancel.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    if let Ok(frame) = media::thumbnail(&asset, at, 360, cancel.clone())
                        && send.send((index, frame)).is_err()
                    {
                        break;
                    }
                }
            })
            .ok();
    }
    fn drain(&mut self) {
        let Some(receive) = &self.receive else { return };
        while let Ok((index, frame)) = receive.try_recv() {
            let size = [frame.width as usize, frame.height as usize];
            if frame.pixels.len() == size[0].saturating_mul(size[1]).saturating_mul(4)
                && let Some(slot) = self.images.get_mut(index)
            {
                *slot = Some(egui::ColorImage::from_rgba_unmultiplied(
                    size,
                    &frame.pixels,
                ));
            }
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.thumbs.stop();
        if let Some(job) = &self.generating {
            job.cancel.cancel();
            job.runner_cancel.cancel();
        }
        if let Some(batch) = &self.batch {
            batch.cancel_all();
        }
    }
}
impl State {
    pub fn new(preferences: Preferences, budget: ByteBudget) -> Result<Self> {
        Self::with_generator(
            preferences,
            budget,
            Arc::new(super::workflow::LocalVideoWorkflow),
            Arc::new(Files),
        )
    }
    pub fn with_generator(
        preferences: Preferences,
        budget: ByteBudget,
        generator: Arc<dyn Generator>,
        preview_sources: Arc<dyn Sources>,
    ) -> Result<Self> {
        Ok(Self {
            preferences,
            request: GenerationRequest {
                path: PathBuf::new(),
                language: "vi".into(),
                transcription_provider: "whisper.cpp".into(),
                transcription_model: "whisper-large-v3-turbo".into(),
                suggestion_provider: "heuristic".into(),
                suggestion_model: "sentence-pause-keyword".into(),
                count: 5,
                min_seconds: 20,
                max_seconds: 60,
                max_segments: 4,
            },
            source: None,
            reviews: Vec::new(),
            selected: 0,
            undo: VecDeque::new(),
            redo: Vec::new(),
            crop_entry: None,
            crop_cancelled: false,
            crop_focused: false,
            trim_drafts: Vec::new(),
            draft_locale: None,
            preview: None,
            preview_dirty: false,
            preview_index: None,
            preview_sources,
            generator,
            generating: None,
            runner: JobRunner::new(
                RunnerConfig {
                    concurrency: [2, 1, 1],
                    queue: 32,
                    retained: 64,
                    network_attempts: 1,
                    backoff: Duration::from_millis(10),
                    progress_interval: Duration::from_millis(20),
                },
                budget.clone(),
                Arc::new(SystemClock::default()),
            )?,
            batch: None,
            budget,
            picker: None,
            error: None,
            error_text: Text::ShortsError,
            cancelled: false,
            captions: true,
            tool: Some(super::view::Tool::Captions),
            list_open: true,
            options_open: false,
            frame: None,
            editing: false,
            thumbs: Thumbs::default(),
            configured: false,
        })
    }
    pub(super) fn pick(&mut self, ctx: &egui::Context, kind: Pick) {
        let future: std::pin::Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>>>> =
            match kind {
                Pick::Video => Box::pin(
                    rfd::AsyncFileDialog::new()
                        .add_filter(
                            catalog::text(self.preferences.locale, Text::Video),
                            &["mp4", "mov", "mkv", "webm", "m4v"],
                        )
                        .pick_file(),
                ),
                Pick::Export => Box::pin(rfd::AsyncFileDialog::new().pick_folder()),
            };
        self.picker = Some(Picker {
            future,
            kind,
            waker: std::task::Waker::from(Arc::new(Repaint(ctx.clone()))),
        });
    }
    fn poll_picker(&mut self) {
        let Some(mut picker) = self.picker.take() else {
            return;
        };
        match picker
            .future
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&picker.waker))
        {
            std::task::Poll::Pending => self.picker = Some(picker),
            std::task::Poll::Ready(None) => {}
            std::task::Poll::Ready(Some(file)) => {
                let result = match picker.kind {
                    Pick::Video => self.pick_video(file.path().to_owned()),
                    Pick::Export => self.export_to(file.path().to_owned()),
                };
                if let Err(e) = result {
                    self.fail(e, Text::ShortsError);
                }
            }
        }
    }
    pub fn reviews(&self) -> &[Review] {
        &self.reviews
    }
    pub fn source(&self) -> Option<&ReviewSource> {
        self.source.as_ref()
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    pub fn error(&self) -> Option<&Error> {
        self.error.as_ref()
    }
    pub fn preview(&self) -> Option<&Editor> {
        self.preview.as_ref()
    }
    pub fn preview_mut(&mut self) -> Option<&mut Editor> {
        self.preview.as_mut()
    }
    pub fn history_len(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn busy(&self) -> bool {
        self.generating.is_some() || self.exporting() || self.picker.is_some()
    }
    pub fn cancelling(&self) -> bool {
        self.generating
            .as_ref()
            .is_some_and(|job| job.cancel.cancelled())
    }
    pub fn awaiting_consent(&self) -> bool {
        self.generating.as_ref().is_some_and(|job| {
            !job.cancel.cancelled()
                && job
                    .events
                    .state
                    .lock()
                    .is_ok_and(|state| state.upload.is_some())
        })
    }
    pub fn exporting(&self) -> bool {
        self.batch.as_ref().is_some_and(|batch| {
            (0..batch.len()).any(|i| {
                batch
                    .snapshot(&self.runner, i)
                    .is_some_and(|s| !terminal(s.state))
            })
        })
    }
    /// Picker and drop injection share this path; replacement refuses during work.
    pub fn pick_video(&mut self, path: PathBuf) -> Result<()> {
        if self.busy() {
            return Err(Error::Invalid("shorts operation in progress"));
        }
        if path.as_os_str().is_empty() || path.as_os_str().len() > 32768 {
            return Err(Error::Invalid("video path"));
        }
        self.request.path = path;
        self.crop_entry = None;
        self.crop_cancelled = false;
        self.crop_focused = false;
        self.source = None;
        self.reviews.clear();
        self.editing = false;
        self.thumbs.stop();
        self.preview = None;
        self.batch = None;
        self.undo.clear();
        self.redo.clear();
        self.trim_drafts.clear();
        self.error = None;
        self.cancelled = false;
        Ok(())
    }
    pub fn generate(&mut self) -> Result<()> {
        if self.busy() || self.request.path.as_os_str().is_empty() {
            return Err(Error::Invalid("pick a video before generating"));
        }
        if !(1..=32).contains(&self.request.count)
            || self.request.min_seconds == 0
            || self.request.min_seconds > self.request.max_seconds
            || self.request.max_seconds > 180
            || !(1..=32).contains(&self.request.max_segments)
        {
            return Err(Error::Invalid("shorts duration or count"));
        }
        self.preview = None;
        let cancel = Cancellation::new();
        let events = Arc::new(TaskEvents::new(cancel.clone()));
        let worker_events = events.clone();
        let worker_cancel = cancel.clone();
        let request = self.request.clone();
        let generator = self.generator.clone();
        let budget = self.budget.clone();
        let (send, receive) = mpsc::sync_channel(1);
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Preview,
            source: [0; 32],
            provider: "desktop",
            model: "shorts-workflow",
            version: "1",
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let (id, runner_cancel) = self.runner.submit(
            JobSpec {
                kind: JobKind::SuggestClips,
                class: ResourceClass::Cpu,
                input: key,
                bytes: Default::default(),
            },
            move |ctx| {
                ctx.check_cancelled()?;
                if worker_cancel.cancelled() {
                    return Err(engine::EditError::Cancelled);
                }
                match generator.run(
                    request.clone(),
                    budget.clone(),
                    &worker_events,
                    &worker_cancel,
                ) {
                    Ok(source) => {
                        send.send(Ok(source))
                            .map_err(|_| engine::EditError::Cancelled)?;
                        Ok(Vec::new())
                    }
                    Err(error) => {
                        let error = Arc::new(error);
                        send.send(Err(Error::Worker(error.clone())))
                            .map_err(|_| engine::EditError::Cancelled)?;
                        Err(engine::EditError::JobFailure(error))
                    }
                }
            },
        )?;
        self.generating = Some(Generating {
            id,
            runner_cancel,
            cancel,
            events,
            receive,
        });
        self.error = None;
        self.cancelled = false;
        Ok(())
    }
    pub fn cancel(&mut self) {
        if let Some(job) = &self.generating {
            job.cancel.cancel();
            job.runner_cancel.cancel();
        }
        if let Some(batch) = &self.batch {
            batch.cancel_all();
        }
    }
    pub fn approve_upload(&mut self, approve: bool) -> Result<()> {
        let job = self
            .generating
            .as_ref()
            .ok_or(Error::Invalid("no generation"))?;
        let upload = job
            .events
            .state
            .lock()
            .map_err(|_| Error::Poisoned)?
            .upload
            .take()
            .ok_or(Error::Invalid("no upload request"))?;
        upload.reply.send(approve).map_err(|_| Error::Cancelled)
    }
    pub fn tick(&mut self) {
        self.thumbs.drain();
        if self.draft_locale != Some(self.preferences.locale) {
            self.reset_drafts();
        }
        self.poll_picker();
        if let Err(e) = self.runner.tick() {
            self.fail(e.into(), Text::ShortsError);
        }
        self.runner.drain_events();
        let done = self
            .generating
            .as_ref()
            .and_then(|job| self.runner.snapshot(job.id))
            .is_some_and(|s| terminal(s.state));
        if done && let Some(job) = self.generating.take() {
            if job.cancel.cancelled() {
                self.cancelled = true;
            } else {
                let result = job.receive.try_recv().unwrap_or_else(|_| {
                    Err(self
                        .runner
                        .snapshot(job.id)
                        .and_then(|s| s.error)
                        .map(Error::Worker)
                        .unwrap_or(Error::Invalid("missing shorts result")))
                });
                match result {
                    Ok(source) => {
                        if let Err(e) = self.install(source) {
                            self.fail(e, Text::ShortsError);
                        }
                    }
                    Err(e) => self.fail(e, Text::ShortsError),
                }
            }
            let _ = self.runner.take_output(job.id);
        }
        if self.preview_dirty {
            self.preview_dirty = false;
            if let Err(e) = self.refresh_preview() {
                self.fail(e, Text::ShortsError);
            }
        }
        if let Some(preview) = &mut self.preview {
            preview.tick();
        }
    }
    pub(super) fn install(&mut self, source: ReviewSource) -> Result<()> {
        if source.candidates.len() > 32 {
            return Err(Error::Invalid("shorts candidate limit"));
        }
        let asset = source
            .project
            .data()
            .assets
            .iter()
            .find(|a| a.id() == source.asset)
            .ok_or(Error::Invalid("shorts source asset"))?;
        let stream = asset
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == media::StreamKind::Video)
            .ok_or(Error::Invalid("shorts needs video"))?;
        let geometry = stream
            .video
            .as_ref()
            .ok_or(Error::Invalid("shorts video dimensions"))?;
        let center = crop::portrait(f64::from(geometry.width), f64::from(geometry.height))?;
        let reviews: Vec<_> = source
            .candidates
            .iter()
            .map(|c| Review {
                segments: c.segments.clone(),
                crop: center,
                decision: Decision::Pending,
            })
            .collect();
        // Admission validates all candidates before changing the current review.
        for (candidate, review) in source.candidates.iter().zip(&reviews) {
            let req = ClipBatchRequest::new(
                source.asset,
                vec![
                    ClipShort::new(candidate.proposal.title.clone(), review.segments.clone())
                        .with_crop(Some(review.crop)),
                ],
                std::env::temp_dir(),
            );
            ClipBatch::prepare(&source.project, req)?;
        }
        let starts = source
            .candidates
            .iter()
            .map(|c| c.segments.first().map_or(Tick::ZERO, |s| s.range.start()))
            .collect();
        let poster_asset = source
            .project
            .data()
            .assets
            .iter()
            .find(|a| a.id() == source.asset)
            .cloned();
        if let Some(asset) = poster_asset {
            self.thumbs.start(asset, starts);
        }
        self.editing = false;
        self.source = Some(source);
        self.reviews = reviews;
        self.selected = 0;
        self.undo.clear();
        self.redo.clear();
        self.batch = None;
        self.error = None;
        self.reset_drafts();
        self.preview_dirty = true;
        Ok(())
    }
    pub(super) fn fail(&mut self, error: Error, text: Text) {
        self.error_text = if matches!(text, Text::ShortsError) {
            match &error {
                Error::Engine(e) => failure_text(e),
                Error::Worker(e) => failure_text(e),
                _ => text,
            }
        } else {
            text
        };
        self.error = Some(error);
    }
    pub(super) fn reset_drafts(&mut self) {
        self.draft_locale = Some(self.preferences.locale);
        self.trim_drafts = self
            .reviews
            .get(self.selected)
            .map(|r| {
                r.segments
                    .iter()
                    .map(|s| {
                        (
                            seconds(self.preferences.locale, s.range.start()),
                            seconds(self.preferences.locale, s.range.end()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    pub fn select(&mut self, index: usize) -> Result<()> {
        if index >= self.reviews.len() {
            return Err(Error::Invalid("candidate index"));
        }
        if self.selected != index {
            self.selected = index;
            self.crop_entry = None;
            self.crop_cancelled = false;
            self.crop_focused = false;
            self.reset_drafts();
            self.preview_dirty = true;
        }
        Ok(())
    }
    pub(super) fn commit(&mut self, index: usize, after: Review) -> Result<()> {
        let before = self
            .reviews
            .get(index)
            .cloned()
            .ok_or(Error::Invalid("candidate index"))?;
        if before != after {
            *self
                .reviews
                .get_mut(index)
                .ok_or(Error::Invalid("candidate index"))? = after.clone();
            let mut picture = before.clone();
            picture.decision = after.decision;
            self.preview_dirty |= picture != after;
            self.undo.push_back(Edit {
                index,
                before,
                after,
            });
            if self.undo.len() > 100 {
                self.undo.pop_front();
            }
            self.redo.clear();
        }
        Ok(())
    }
    pub fn decide(&mut self, decision: Decision) -> Result<()> {
        let mut row = self
            .reviews
            .get(self.selected)
            .cloned()
            .ok_or(Error::Invalid("no candidate"))?;
        row.decision = decision;
        self.commit(self.selected, row)
    }
    /// Safe frame/word snapping is owned by engine; failure leaves review/history intact.
    pub fn trim(&mut self, segment: usize, start: Tick, end: Tick) -> Result<()> {
        let source = self.source.as_ref().ok_or(Error::Invalid("no source"))?;
        let mut row = self
            .reviews
            .get(self.selected)
            .cloned()
            .ok_or(Error::Invalid("no candidate"))?;
        row.segments
            .get_mut(segment)
            .ok_or(Error::Invalid("segment index"))?
            .range = SourceRange::new(start, end)?;
        let profile = captions::LanguageRegistry::standard()
            .map_err(engine::EditError::from)?
            .profile(&source.transcript.source_lang)
            .clone();
        let snapped = snap_clip_boundaries(
            &[row.segments],
            &source.transcript,
            &profile,
            &captions::LayoutHooks::default(),
            &source.energy,
            &source.boundaries,
        )?;
        row.segments = snapped
            .into_iter()
            .next()
            .ok_or(Error::Invalid("no snapped short"))?
            .segments;
        self.commit(self.selected, row)?;
        self.reset_drafts();
        Ok(())
    }
    pub fn set_crop(&mut self, crop: render::Rect) -> Result<()> {
        let mut row = self
            .reviews
            .get(self.selected)
            .cloned()
            .ok_or(Error::Invalid("no candidate"))?;
        if (crop.width - row.crop.width).abs() > 1e-9
            || (crop.height - row.crop.height).abs() > 1e-9
        {
            return Err(Error::Invalid("portrait crop dimensions"));
        }
        row.crop = crop;
        self.prepare(&[self.selected], Some(&row), std::env::temp_dir())?;
        self.commit(self.selected, row)
    }
    pub fn undo(&mut self) {
        if let Some(edit) = self.undo.pop_back() {
            if let Some(row) = self.reviews.get_mut(edit.index) {
                *row = edit.before.clone();
            }
            self.selected = edit.index;
            self.redo.push(edit);
            self.reset_drafts();
            self.preview_dirty = true;
        }
    }
    pub fn redo(&mut self) {
        if let Some(edit) = self.redo.pop() {
            if let Some(row) = self.reviews.get_mut(edit.index) {
                *row = edit.after.clone();
            }
            self.selected = edit.index;
            self.undo.push_back(edit);
            self.reset_drafts();
            self.preview_dirty = true;
        }
    }
    pub(super) fn prepare(
        &self,
        indices: &[usize],
        override_row: Option<&Review>,
        directory: PathBuf,
    ) -> Result<ClipBatch> {
        let source = self.source.as_ref().ok_or(Error::Invalid("no source"))?;
        let mut shorts = Vec::with_capacity(indices.len());
        for index in indices {
            let review = override_row
                .or_else(|| self.reviews.get(*index))
                .ok_or(Error::Invalid("candidate index"))?;
            let candidate = source
                .candidates
                .get(*index)
                .ok_or(Error::Invalid("candidate index"))?;
            shorts.push(
                ClipShort::new(candidate.proposal.title.clone(), review.segments.clone())
                    .with_crop(Some(review.crop)),
            );
        }
        let mut request = ClipBatchRequest::new(source.asset, shorts, directory);
        if self.captions {
            request.cues = source.cues.clone();
            request.languages = vec![source.transcript.source_lang.clone()];
            request.subtitle_formats = vec![captions::Format::Srt];
        }
        Ok(ClipBatch::prepare(&source.project, request)?)
    }
    pub(super) fn refresh_preview(&mut self) -> Result<()> {
        if self.reviews.is_empty() {
            self.preview = None;
            return Ok(());
        }
        let batch = self.prepare(&[self.selected], None, std::env::temp_dir())?;
        let project = batch
            .project(0)
            .ok_or(Error::Invalid("preview project"))?
            .clone();
        let settings = batch
            .settings(0)
            .ok_or(Error::Invalid("preview settings"))?
            .render
            .clone();
        let session = Session::new(project, 100)?;
        if let Some(editor) = &mut self.preview {
            let at = if self.preview_index == Some(self.selected) {
                editor
                    .playback
                    .position()?
                    .min(session.project().duration())
            } else {
                Tick::ZERO
            };
            editor.playback.seek(at, false, session.project().clone())?;
            editor.playback.set_render_settings(settings)?;
            editor.session = crate::lifecycle::EditorSession::Memory(Box::new(session));
        } else {
            let mut playback =
                Playback::new(&session, self.budget.clone(), self.preview_sources.clone())?;
            playback.set_render_settings(settings)?;
            self.preview = Some(Editor::new(session, playback, self.preferences.clone()));
        }
        self.preview_index = Some(self.selected);
        Ok(())
    }
    /// Accept only explicitly approved shorts. /05 owns jobs, names and output admission.
    pub fn export_to(&mut self, directory: PathBuf) -> Result<()> {
        if self.busy() {
            return Err(Error::Invalid("shorts operation in progress"));
        }
        let indices: Vec<_> = self
            .reviews
            .iter()
            .enumerate()
            .filter(|(_, r)| r.decision == Decision::Accepted)
            .map(|(i, _)| i)
            .collect();
        if indices.is_empty() {
            return Err(Error::Invalid("no accepted shorts"));
        }
        let mut batch = self.prepare(&indices, None, directory)?;
        let result = batch.submit(&mut self.runner);
        self.batch = Some(batch); // Retain partial admission so every job remains cancellable.
        result?;
        self.error = None;
        Ok(())
    }
    pub fn batch(&self) -> Option<&ClipBatch> {
        self.batch.as_ref()
    }
    pub fn batch_snapshot(&self, index: usize) -> Option<engine::jobs::JobSnapshot> {
        self.batch.as_ref()?.snapshot(&self.runner, index)
    }
    pub fn retry_export(&mut self, index: usize) -> Result<()> {
        self.batch
            .as_mut()
            .ok_or(Error::Invalid("no export batch"))?
            .retry(&mut self.runner, index)?;
        Ok(())
    }
    /// A fresh portrait project, with all segments/captions inserted by one command
    /// transaction. Undo removes the insertion, preserving the imported source asset.
    pub fn editor_session(&self) -> Result<(Session, render::RenderSettings)> {
        let batch = self.prepare(&[self.selected], None, std::env::temp_dir())?;
        let prepared = batch.project(0).ok_or(Error::Invalid("editor project"))?;
        let mut data = prepared.data().clone();
        data.id = crate::workspace::fresh(prepared.data().name.as_str())?
            .project()
            .data()
            .id;
        data.canvas.width = 1080;
        data.canvas.height = 1920;
        // The prepared short already contains its first visual media. Retain
        // that baseline while replaying its items into the portrait editor.
        data.original_canvas_size = data.original_canvas_size.or_else(|| {
            prepared.data().assets.iter().find_map(|asset| {
                asset.probe().streams.iter().find_map(|stream| {
                    stream.video.as_ref().map(|video| project::CanvasSize {
                        width: video.width,
                        height: video.height,
                    })
                })
            })
        });
        data.scene_mut().timeline = project::Timeline::default();
        let mut session = Session::new(project::Project::new(data)?, 100)?;
        session.transaction(|s| {
            for track in prepared.data().scene().timeline.iter() {
                let target = if track.kind == project::TrackKind::Video {
                    s.project().data().scene().timeline.main.id
                } else {
                    s.apply(Command::AddTrack {
                        placement: engine::TrackPlacement::Auto,
                        name: track.name.clone(),
                        kind: track.kind,
                    })?
                    .created
                    .into_iter()
                    .find_map(|id| {
                        if let engine::CreatedId::Track(id) = id {
                            Some(id)
                        } else {
                            None
                        }
                    })
                    .ok_or(engine::EditError::Invalid("created track"))?
                };
                for item in &track.items {
                    s.apply(Command::InsertItem {
                        placement: engine::Placement::Track { track: target },
                        item: NewItem {
                            retime: item.retime,
                            animations: item.animations.clone(),
                            source_asset: item.source_asset,
                            source_range: item.source_range,
                            start: item.start,
                            content: item.content.clone(),
                        },
                    })?;
                }
            }
            Ok(())
        })?;
        let mut settings = batch
            .settings(0)
            .ok_or(Error::Invalid("editor render settings"))?
            .render
            .clone();
        // Commands allocate fresh IDs. Preserve placements by video play order.
        let placements: Vec<_> = prepared
            .data()
            .scene()
            .timeline
            .main
            .items
            .iter()
            .filter_map(|i| settings.placements.get(&i.id).copied())
            .collect();
        settings.placements.clear();
        for (item, placement) in session
            .project()
            .data()
            .scene()
            .timeline
            .main
            .items
            .iter()
            .zip(placements)
        {
            settings.placements.insert(item.id, placement);
        }
        Ok((session, settings))
    }
}
pub(super) fn terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Failed | JobState::Cancelled
    )
}
pub(super) fn seconds(locale: Locale, tick: Tick) -> String {
    catalog::decimal(locale, tick.0 as f64 / TICKS_PER_SECOND as f64, 3)
}
pub(super) fn parse_seconds(locale: Locale, value: &str) -> Result<Tick> {
    let value = catalog::parse_decimal(locale, value)?;
    if !(0.0..=10800.0).contains(&value) {
        return Err(Error::Invalid("trim seconds"));
    }
    Ok(Tick((value * TICKS_PER_SECOND as f64).round() as i64))
}

pub(super) fn failure_text(error: &engine::EditError) -> Text {
    match error {
        engine::EditError::ModelMissing(_) | engine::EditError::RunnerMissing => {
            Text::ShortsModelMissing
        }
        engine::EditError::CredentialMissing(_) => Text::ShortsCredentialsMissing,
        engine::EditError::ConsentRequired => Text::ShortsConsentRequired,
        engine::EditError::BudgetFull | engine::EditError::Render(render::Error::Budget { .. }) => {
            Text::ShortsMemory
        }
        engine::EditError::Render(render::Error::Unavailable(_)) => Text::ShortsSourceUnavailable,
        engine::EditError::Io(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Text::ShortsOutputExists
        }
        engine::EditError::Project(project::Error::Io(e))
        | engine::EditError::Render(
            render::Error::Io(e) | render::Error::Project(project::Error::Io(e)),
        ) if e.kind() == std::io::ErrorKind::AlreadyExists => Text::ShortsOutputExists,
        _ => Text::ShortsError,
    }
}
