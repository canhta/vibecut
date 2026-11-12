//! Retains review state for accepted shorts.
use crate::{
    Error, Result,
    appearance::Look,
    egui,
    i18n::{self, Locale, Text},
    services::{
        ai_access::{self, Choice, Opener},
        crop,
        playback::{Files, Playback, Sources, Transport},
        preview,
    },
};
use engine::ClipSourceSegment;
use engine::{
    ByteBudget, Command, Session,
    control::{ClipExportShort, ControlResult},
    jobs::{JobId, JobRunner, JobState, RunnerConfig, SystemClock},
    subtitles::CaptionAccess,
    transcript::Cancellation,
};
pub use shorts::ClipCandidate as Candidate;
use project::ItemParams;
use shorts::{
    ClipBatch, ClipBatchRequest, ClipShort,
    clip_boundaries::{ClipBoundarySettings, snap_clip_boundaries},
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
    pub crop: project::Crop,
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
pub use crate::services::ai_task::TaskEvents;
pub(super) struct Generating {
    pub(super) id: JobId,
    pub(super) cancel: Cancellation,
    pub(super) events: Arc<TaskEvents>,
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

/// The selected short as the window plays it back: the prepared snapshot, its
/// transport and the uploaded frame.
pub(super) struct Preview {
    pub(super) session: Session,
    pub(super) playback: Playback,
    pub(super) frame: preview::FrameTexture,
}

pub struct State {
    pub(crate) models: crate::services::remote_models::AiPreferences,
    pub preferences: Look,
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
    pub(super) preview: Option<Preview>,
    pub(super) preview_dirty: bool,
    pub(super) preview_index: Option<usize>,
    pub(super) preview_sources: Arc<dyn Sources>,
    /// Builds the access one run lends to the registry; tests inject fixture providers.
    pub(super) access: Opener,
    pub(super) generating: Option<Generating>,
    /// The source session suggestion and export commands run in, with its caption item.
    pub(super) session: Option<Session>,
    pub(super) asset: Option<media::SourceAsset>,
    pub(super) duration: Tick,
    pub(super) run_access: Option<CaptionAccess>,
    /// The retained `clips.suggest` job the export command decodes.
    pub(super) suggestion: Option<JobId>,
    // Cancellation in Drop runs before runner joins an approval-waiting worker.
    pub(super) runner: JobRunner,
    /// One entry per accepted short: its own export job and published paths.
    pub(super) exports: Option<Vec<ClipExportShort>>,
    pub(super) budget: ByteBudget,
    pub(super) picker: Option<Picker>,
    pub(super) error: Option<Error>,
    pub(super) error_text: Text,
    pub(super) cancelled: bool,
    pub captions: bool,
    pub(super) tool: Option<super::view::Tool>,
    pub(super) list_open: bool,
    pub(super) options_open: bool,
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
        self.cancel();
    }
}
impl State {
    pub(crate) fn apply_model_preferences(
        &mut self,
        models: &crate::services::remote_models::AiPreferences,
    ) {
        self.models = models.clone();
        if matches!(
            self.request.transcription_provider.as_str(),
            "openai" | "elevenlabs"
        ) {
            self.request.transcription_model = models
                .model(
                    &self.request.transcription_provider,
                    ai::Operation::Transcribe,
                )
                .into();
        }
        if matches!(
            self.request.suggestion_provider.as_str(),
            "openai" | "anthropic"
        ) {
            self.request.suggestion_model = models
                .model(
                    &self.request.suggestion_provider,
                    ai::Operation::SuggestClips,
                )
                .into();
        }
    }
    pub fn new(preferences: Look, budget: ByteBudget) -> Result<Self> {
        Self::with_access(
            preferences,
            budget.clone(),
            ai_access::desktop_opener(budget),
            Arc::new(Files),
        )
    }
    /// Tests inject the opener at the lent-access seam; the window still sends registry commands.
    pub fn with_access(
        preferences: Look,
        budget: ByteBudget,
        access: Opener,
        preview_sources: Arc<dyn Sources>,
    ) -> Result<Self> {
        Ok(Self {
            models: Default::default(),
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
            access,
            generating: None,
            session: None,
            asset: None,
            duration: Tick::ZERO,
            run_access: None,
            suggestion: None,
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
            exports: None,
            budget,
            picker: None,
            error: None,
            error_text: Text::ShortsError,
            cancelled: false,
            captions: true,
            tool: Some(super::view::Tool::Captions),
            list_open: true,
            options_open: false,
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
                            i18n::text(self.preferences.locale, Text::Video),
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
    pub fn preview(&self) -> Option<&Playback> {
        self.preview.as_ref().map(|preview| &preview.playback)
    }
    pub fn preview_mut(&mut self) -> Option<&mut Playback> {
        self.preview.as_mut().map(|preview| &mut preview.playback)
    }
    /// The prepared project the preview renders.
    pub fn preview_session(&self) -> Option<&Session> {
        self.preview.as_ref().map(|preview| &preview.session)
    }
    /// The tick of the last preview frame uploaded for presentation.
    pub fn presented_frame(&self) -> Option<Tick> {
        self.preview
            .as_ref()
            .and_then(|preview| preview.frame.presented())
    }
    /// Closing the window stops its transport; the project is untouched.
    pub fn pause_preview(&mut self) {
        if let Some(preview) = &mut self.preview {
            let project = preview.session.project().clone();
            let _ = preview.playback.transport(Transport::Pause, project);
        }
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
        self.exports.as_ref().is_some_and(|exports| {
            exports.iter().any(|short| {
                self.runner
                    .snapshot(short.job)
                    .is_some_and(|s| !terminal(s.state))
            })
        })
    }
    /// Progress of the running suggestion job, 0..=1.
    pub fn generation_progress(&self) -> Option<f32> {
        let job = self.generating.as_ref()?;
        self.runner
            .snapshot(job.id)
            .map(|snapshot| snapshot.progress.clamp(0.0, 1.0))
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
        self.reset_source();
        self.cancelled = false;
        Ok(())
    }
    fn reset_source(&mut self) {
        self.crop_entry = None;
        self.crop_cancelled = false;
        self.crop_focused = false;
        self.source = None;
        self.reviews.clear();
        self.editing = false;
        self.thumbs.stop();
        self.preview = None;
        self.exports = None;
        self.session = None;
        self.asset = None;
        self.duration = Tick::ZERO;
        self.run_access = None;
        self.suggestion = None;
        self.undo.clear();
        self.redo.clear();
        self.trim_drafts.clear();
        self.error = None;
    }
    /// One registry job: import the picked source, then ask `clips.suggest` for candidates.
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
        let asset = media::SourceAsset::import(media::AssetId(1), self.request.path.clone())
            .map_err(|error| Error::Engine(engine::control::Error::Media(error)))?;
        let duration = asset
            .probe()
            .duration
            .filter(|duration| *duration > Tick::ZERO)
            .ok_or(Error::Invalid("shorts source duration"))?;
        let mut session = engine::transcript::source_session(&asset, duration, true, None)?;
        let cancel = Cancellation::new();
        let events = Arc::new(TaskEvents::new(cancel.clone()));
        let access = (self.access)(
            Choice::Transcribe {
                provider: &self.request.transcription_provider,
                model: &self.request.transcription_model,
            },
            events.clone(),
        )?;
        let language = engine::providers::parse_language(&self.request.language)?;
        let min = Tick(i64::from(self.request.min_seconds) * TICKS_PER_SECOND);
        let max = Tick(i64::from(self.request.max_seconds) * TICKS_PER_SECOND);
        let (id, _) = ai_access::accepted(ai_access::run(
            &mut session,
            &mut self.runner,
            Some(&access),
            None,
            "clips.suggest",
            serde_json::json!({
                "asset": asset.id(),
                "source_language": language,
                "provider": self.request.suggestion_provider,
                "model": self.request.suggestion_model,
                "count": self.request.count,
                "min_duration": min,
                "max_duration": max,
                "max_segments": self.request.max_segments,
                "min_segment_duration": Tick(TICKS_PER_SECOND),
                "overlap_tolerance": 0.2,
                "keywords": Vec::<String>::new(),
            }),
        )?)?;
        self.session = Some(session);
        self.asset = Some(asset);
        self.duration = duration;
        self.run_access = Some(access);
        self.suggestion = Some(id);
        self.exports = None;
        self.generating = Some(Generating { id, cancel, events });
        self.error = None;
        self.cancelled = false;
        Ok(())
    }
    pub fn cancel(&mut self) {
        if let Some(job) = &self.generating {
            job.cancel.cancel();
            let _ = self.runner.cancel_job(job.id);
        }
        if let Some(exports) = &self.exports {
            for short in exports {
                let _ = self.runner.cancel_job(short.job);
            }
        }
    }
    pub fn approve_upload(&mut self, approve: bool) -> Result<()> {
        let job = self
            .generating
            .as_ref()
            .ok_or(Error::Invalid("no generation"))?;
        job.events.answer(approve)
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
            let snapshot = self.runner.snapshot(job.id);
            let state = snapshot.as_ref().map_or(JobState::Failed, |s| s.state);
            if job.cancel.cancelled() || state == JobState::Cancelled {
                self.cancelled = true;
                let _ = self.runner.take_output(job.id);
            } else if state == JobState::Completed {
                // Keep the retained suggestion: `clips.export` decodes the same job.
                if let Err(e) = self.finish_suggestion(job.id) {
                    self.fail(e, Text::ShortsError);
                }
            } else {
                let error = snapshot
                    .and_then(|s| s.error)
                    .map(Error::Worker)
                    .unwrap_or_else(|| Error::Invalid("missing shorts result"));
                self.fail(error, Text::ShortsError);
                let _ = self.runner.take_output(job.id);
            }
        }
        if self.preview_dirty {
            self.preview_dirty = false;
            if let Err(e) = self.refresh_preview() {
                self.fail(e, Text::ShortsError);
            }
        }
        if let Some(preview) = &mut self.preview
            && let Err(e) = preview.playback.poll(std::time::Instant::now())
        {
            self.fail(e, Text::ShortsError);
        }
    }
    /// Decode the retained suggestion, rebuild the source session with its caption item and
    /// install the review. The job output stays retained for `clips.export`.
    fn finish_suggestion(&mut self, id: JobId) -> Result<()> {
        let bytes = self
            .runner
            .output_bytes(id)
            .ok_or(Error::Invalid("shorts suggestion missing"))?;
        let result = engine::control::decode_clips(bytes).map_err(Error::Engine)?;
        let asset = self.asset.clone().ok_or(Error::Invalid("no shorts source"))?;
        let duration = self.duration;
        let session =
            engine::transcript::build_session(&asset, &result.cues, duration, true, None)?;
        let project = session.project().as_ref().clone();
        let transcript = transcript_from(&result);
        let boundaries = review_boundaries(&asset, duration, &self.request)?;
        let cues = captions::CueTrack {
            styles: Vec::new(),
            cues: result.cues.clone(),
        };
        let candidates = result
            .clips
            .iter()
            .map(candidate_from_view)
            .collect::<Result<Vec<_>>>()?;
        self.session = Some(session);
        self.install(ReviewSource {
            project,
            asset: result.asset,
            transcript,
            cues,
            // ponytail: clips.suggest does not carry the RMS hint; word/segment snapping
            // remains exact. Add a review field to the wire when trim needs silence hints.
            energy: Vec::new(),
            boundaries,
            candidates,
        })
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
        self.exports = None;
        self.error = None;
        self.reset_drafts();
        self.preview_dirty = true;
        Ok(())
    }
    pub(super) fn fail(&mut self, error: Error, text: Text) {
        self.error_text = if matches!(text, Text::ShortsError) {
            match &error {
                Error::Engine(e) => failure_text(e),
                Error::Worker(e) => failure_text(e.as_ref()),
                Error::Shorts(shorts::Error::Engine(e)) => failure_text(e),
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
            .map_err(engine::control::Error::from)?
            .profile(&source.transcript.source_lang)
            .clone();
        let snapped = snap_clip_boundaries(
            &[row.segments],
            &source.transcript,
            &profile,
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
    pub fn set_crop(&mut self, crop: project::Crop) -> Result<()> {
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
        if let Some(preview) = &mut self.preview {
            let at = if self.preview_index == Some(self.selected) {
                preview
                    .playback
                    .position()?
                    .min(session.project().duration())
            } else {
                Tick::ZERO
            };
            preview
                .playback
                .seek(at, false, session.project().clone())?;
            preview.playback.set_render_settings(settings)?;
            preview.session = session;
        } else {
            let mut playback =
                Playback::new(&session, self.budget.clone(), self.preview_sources.clone())?;
            playback.set_render_settings(settings)?;
            self.preview = Some(Preview {
                session,
                playback,
                frame: preview::FrameTexture::default(),
            });
        }
        self.preview_index = Some(self.selected);
        Ok(())
    }
    /// Accept only explicitly approved shorts: one `clips.export` command starts their jobs.
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
        self.exports = Some(self.submit_export(&indices, directory, false)?);
        self.error = None;
        Ok(())
    }
    /// Submit `clips.export` for exactly these candidates; the runner bounds the selection.
    fn submit_export(
        &mut self,
        indices: &[usize],
        directory: PathBuf,
        overwrite: bool,
    ) -> Result<Vec<ClipExportShort>> {
        let suggestion = self.suggestion.ok_or(Error::Invalid("no clip suggestion"))?;
        let access = self
            .run_access
            .clone()
            .ok_or(Error::Invalid("no clip access"))?;
        let source = self.source.as_ref().ok_or(Error::Invalid("no source"))?;
        let languages = if self.captions {
            vec![source.transcript.source_lang.clone()]
        } else {
            Vec::new()
        };
        let formats = if self.captions {
            vec![captions::Format::Srt]
        } else {
            Vec::new()
        };
        let item = if self.captions {
            caption_item(&self.session)?
        } else {
            None
        };
        let params = serde_json::json!({
            "job": suggestion,
            "select": indices,
            "item": item,
            "languages": languages,
            "formats": formats,
            "directory": directory,
            "overwrite": overwrite,
            "allow_degraded": false,
        });
        let session = self.session.as_mut().ok_or(Error::Invalid("no source"))?;
        match ai_access::run(
            session,
            &mut self.runner,
            Some(&access),
            None,
            "clips.export",
            params,
        )? {
            ControlResult::ClipExports { shorts, .. } => Ok(shorts),
            _ => Err(Error::Invalid("clip export was not accepted")),
        }
    }
    pub fn export_snapshot(&self, index: usize) -> Option<engine::jobs::JobSnapshot> {
        let short = self.exports.as_ref()?.get(index)?;
        self.runner.snapshot(short.job)
    }
    pub fn export(&self, index: usize) -> Option<&ClipExportShort> {
        self.exports.as_ref()?.get(index)
    }
    pub fn export_count(&self) -> usize {
        self.exports.as_ref().map_or(0, Vec::len)
    }
    pub fn retry_export(&mut self, index: usize) -> Result<()> {
        let candidate = self
            .exports
            .as_ref()
            .and_then(|exports| exports.get(index))
            .map(|short| short.candidate)
            .ok_or(Error::Invalid("no export batch"))?;
        let directory = self
            .exports
            .as_ref()
            .and_then(|exports| exports.get(index))
            .map(|short| {
                short
                    .video
                    .parent()
                    .map_or_else(std::env::temp_dir, std::path::Path::to_path_buf)
            })
            .ok_or(Error::Invalid("no export batch"))?;
        let submitted = self.submit_export(&[candidate], directory, false)?;
        let Some(exports) = self.exports.as_mut() else {
            return Err(Error::Invalid("no export batch"));
        };
        if let Some(short) = submitted.into_iter().next()
            && let Some(slot) = exports.get_mut(index)
        {
            *slot = short;
        }
        Ok(())
    }
    /// A fresh portrait project, with all segments/captions inserted by one command
    /// transaction. Undo removes the insertion, preserving the imported source asset.
    pub fn editor_session(&self) -> Result<(Session, render::RenderSettings)> {
        let batch = self.prepare(&[self.selected], None, std::env::temp_dir())?;
        let prepared = batch.project(0).ok_or(Error::Invalid("editor project"))?;
        let mut data = prepared.data().clone();
        data.id = crate::lifecycle::fresh(prepared.data().name.as_str())?
            .project()
            .data()
            .id;
        data.canvas.width = 1080;
        data.canvas.height = 1920;
        data.canvas.preset = Some(project::AspectPreset::Portrait);
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
        session.transaction(|s| -> std::result::Result<(), engine::EditError> {
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
/// The suggested source's caption item, which `clips.export` burns and maps.
fn caption_item(session: &Option<Session>) -> Result<Option<ItemId>> {
    Ok(session.as_ref().and_then(|session| {
        session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|item| matches!(item.content, ItemParams::Caption(_)))
            .map(|item| item.id)
    }))
}

/// Rebuild the recognition context from the suggestion's source cues. Cues carry the
/// exact native word proof the suggestion was snapped from; no ASR runs again.
fn transcript_from(result: &engine::control::ClipsResult) -> captions::Transcript {
    let segments = result
        .cues
        .iter()
        .filter_map(|cue| {
            let text = cue.texts.get(&cue.source_lang)?;
            let words = match &cue.word_timing {
                captions::CueWordTiming::Words { text, records, .. } => records
                    .iter()
                    .filter_map(|record| {
                        let word = text.as_str().get(record.bytes.clone())?;
                        Some(captions::Word {
                            provenance: Some(record.provenance),
                            text: word.into(),
                            range: Some(record.source_range),
                            speaker: None,
                            confidence: None,
                        })
                    })
                    .collect(),
                captions::CueWordTiming::Unverified => Vec::new(),
            };
            Some(captions::Segment {
                id: cue.id.clone(),
                range: cue.range,
                text: text.text.clone(),
                words,
                spans: None,
            })
        })
        .collect();
    captions::Transcript {
        source_lang: result.source_language.clone(),
        fingerprint: captions::Fingerprint {
            provider: result.transcription_provider.clone().into(),
            model: result.transcription_model.clone().into(),
            settings: "clips.suggest".into(),
        },
        segments,
    }
}

/// Boundary settings of the suggestion command; the same values `clips.suggest` applied.
fn review_boundaries(
    asset: &media::SourceAsset,
    duration: Tick,
    request: &GenerationRequest,
) -> Result<ClipBoundarySettings> {
    let frame_rate = asset
        .probe()
        .streams
        .iter()
        .find(|stream| stream.kind == media::StreamKind::Video)
        .and_then(|stream| stream.frame_rate)
        .unwrap_or(time::FrameRate::FPS_30);
    Ok(ClipBoundarySettings {
        source_range: SourceRange::new(Tick::ZERO, duration)?,
        frame_rate,
        tolerance: Tick(TICKS_PER_SECOND),
        silence_rms: 0.01,
        min_duration: Tick(i64::from(request.min_seconds) * TICKS_PER_SECOND),
        max_duration: Tick(i64::from(request.max_seconds) * TICKS_PER_SECOND),
        min_segment_duration: Tick(TICKS_PER_SECOND),
        max_segments: request.max_segments,
    })
}

fn candidate_from_view(
    view: &engine::control::ClipCandidateView,
) -> Result<Candidate> {
    let role = |name: &str| match name {
        "hook" => Ok(engine::ClipRole::Hook),
        "context" => Ok(engine::ClipRole::Context),
        "payoff" => Ok(engine::ClipRole::Payoff),
        _ => Err(Error::Invalid("clip piece role")),
    };
    let source = match view.source.as_str() {
        "llm" => engine::ClipSource::Llm,
        "heuristic" => engine::ClipSource::Heuristic,
        _ => return Err(Error::Invalid("clip source")),
    };
    let mut pieces = Vec::with_capacity(view.pieces.len());
    for piece in &view.pieces {
        pieces.push(ai::clips::ClipPiece {
            start_segment_id: piece.start_segment_id.clone(),
            end_segment_id: piece.end_segment_id.clone(),
            role: role(piece.role.as_str())?,
        });
    }
    Ok(Candidate {
        proposal: engine::ClipProposal {
            segments: pieces,
            title: view.title.clone(),
            hook_text: view.hook_text.clone(),
            reason: view.reason.clone(),
            score: view.score,
        },
        segments: shorts::suggestion::segments_from_view(view)?,
        // Editors never read the edge deltas; the snapped segments are authoritative.
        adjustments: Vec::new(),
        duration: view.duration,
        source,
    })
}

pub(super) fn terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Failed | JobState::Cancelled
    )
}
pub(super) fn seconds(locale: Locale, tick: Tick) -> String {
    i18n::decimal(locale, tick.0 as f64 / TICKS_PER_SECOND as f64, 3)
}
pub(super) fn parse_seconds(locale: Locale, value: &str) -> Result<Tick> {
    let value = i18n::parse_decimal(locale, value)?;
    if !(0.0..=10800.0).contains(&value) {
        return Err(Error::Invalid("trim seconds"));
    }
    Ok(Tick((value * TICKS_PER_SECOND as f64).round() as i64))
}

pub(super) fn failure_text(error: &(dyn engine::Failure + 'static)) -> Text {
    let cause = error as &(dyn std::error::Error + 'static);
    let root = cause.downcast_ref::<engine::control::Error>();
    let transcript = match root {
        Some(engine::control::Error::Transcript(transcript)) => Some(transcript),
        _ => cause.downcast_ref::<engine::transcript::Error>(),
    };
    let provider = match transcript {
        Some(engine::transcript::Error::Providers(provider)) => Some(provider),
        _ => match root {
            Some(engine::control::Error::Providers(provider)) => Some(provider),
            _ => cause.downcast_ref::<engine::providers::Error>(),
        },
    };
    if let Some(provider) = provider {
        match provider {
            engine::providers::Error::ModelMissing(_) | engine::providers::Error::RunnerMissing => {
                return Text::ShortsModelMissing;
            }
            engine::providers::Error::CredentialMissing(_) => {
                return Text::ShortsCredentialsMissing;
            }
            engine::providers::Error::ConsentRequired => return Text::ShortsConsentRequired,
            _ => {}
        }
    }
    let runtime = match transcript {
        Some(engine::transcript::Error::Runtime(runtime)) => Some(runtime),
        _ => match provider {
            Some(engine::providers::Error::Runtime(runtime)) => Some(runtime),
            _ => match root {
                Some(engine::control::Error::Runtime(runtime)) => Some(runtime),
                _ => cause.downcast_ref::<engine::RuntimeError>(),
            },
        },
    };
    if let Some(runtime) = runtime {
        return match runtime {
            engine::RuntimeError::BudgetFull => Text::ShortsMemory,
            engine::RuntimeError::Io(error)
                if error.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                Text::ShortsOutputExists
            }
            _ => Text::ShortsError,
        };
    }
    let Some(error) = root else {
        return Text::ShortsError;
    };
    match error {
        engine::control::Error::Render(render::Error::Budget { .. }) => Text::ShortsMemory,
        engine::control::Error::Render(render::Error::Unavailable(_)) => {
            Text::ShortsSourceUnavailable
        }
        engine::control::Error::Io(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Text::ShortsOutputExists
        }
        engine::control::Error::Edit(engine::EditError::Project(project::Error::Io(e)))
        | engine::control::Error::Render(
            render::Error::Io(e) | render::Error::Project(project::Error::Io(e)),
        ) if e.kind() == std::io::ErrorKind::AlreadyExists => Text::ShortsOutputExists,
        _ => Text::ShortsError,
    }
}
