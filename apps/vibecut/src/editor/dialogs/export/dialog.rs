//! Presents one export job and its saved choices.
use super::settings;
use crate::i18n;
use crate::services::lifecycle::CaptionOutput;
use crate::services::views::progress::job_progress;
use crate::{
    Error, Result, egui,
    i18n::{Locale, Text},
    kit as theme,
};
use engine::{
    ByteBudget,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{CancelToken, JobId, JobRunner, JobSnapshot, JobState, RunnerConfig, SystemClock},
};
use media::EncodePreference;
pub use render::{OutputOptions, RangeChoice, SceneChoice};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    #[default]
    Mp4,
    Mov,
    Webm,
    /// Audio only: AAC in an MP4 container.
    M4a,
    /// Audio only: Opus in a WebM container.
    AudioWebm,
    /// Audio only: MP3.
    Mp3,
    /// Audio only: 16-bit PCM in a WAV file.
    Wav,
    /// Audio only: FLAC.
    Flac,
    /// Silent animated GIF.
    Gif,
    /// Silent PNG image sequence: a directory of `<name>_%05d.png` frames.
    PngSequence,
}
impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Webm | Self::AudioWebm => "webm",
            Self::M4a => "m4a",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Gif => "gif",
            // A directory, not a file: it has no extension.
            Self::PngSequence => "",
        }
    }
    /// The name an export of `project_name` is saved under: a file name with the
    /// extension, or the bare directory name of an image sequence.
    pub fn file_name(self, project_name: &str) -> String {
        if self.is_sequence() {
            engine::sanitize_file_name(project_name)
        } else {
            default_file_name(project_name, self.extension())
        }
    }
    pub fn is_sequence(self) -> bool {
        self == Self::PngSequence
    }
    /// The container a video format encodes into. An image sequence has none and
    /// reports MP4 only to fill the settings' container field.
    pub fn container(self) -> media::Container {
        match self {
            Self::Mp4 | Self::M4a => media::Container::Mp4,
            Self::Mov => media::Container::Mov,
            Self::Webm | Self::AudioWebm => media::Container::WebM,
            Self::Mp3 => media::Container::Mp3,
            Self::Wav => media::Container::Wav,
            Self::Flac => media::Container::Flac,
            Self::Gif => media::Container::Gif,
            Self::PngSequence => media::Container::Mp4,
        }
    }
    pub fn audio_only(self) -> bool {
        matches!(
            self,
            Self::M4a | Self::AudioWebm | Self::Mp3 | Self::Wav | Self::Flac
        )
    }
    /// Why the video encoder rows (bit rate, codec and H.264 profile) do not apply to
    /// this format, or `None` when they do. Silent image outputs and audio-only output
    /// disable those rows, with this reason on hover.
    pub(super) fn unused_encoder_reason(self) -> Option<Text> {
        match self {
            Self::Gif => Some(Text::ExportNotForGif),
            Self::PngSequence => Some(Text::ExportNotForSequence),
            Self::M4a | Self::AudioWebm | Self::Mp3 | Self::Wav | Self::Flac => {
                Some(Text::ExportNotForAudio)
            }
            Self::Mp4 | Self::Mov | Self::Webm => None,
        }
    }
    pub fn is_gif(self) -> bool {
        self == Self::Gif
    }
    pub(super) fn label(self) -> Text {
        match self {
            Self::Mp4 => Text::ExportMp4,
            Self::Mov => Text::ExportMov,
            Self::Webm => Text::ExportWebm,
            Self::M4a => Text::ExportAudioM4a,
            Self::AudioWebm => Text::ExportAudioWebm,
            Self::Mp3 => Text::ExportAudioMp3,
            Self::Wav => Text::ExportAudioWav,
            Self::Flac => Text::ExportAudioFlac,
            Self::Gif => Text::ExportGif,
            Self::PngSequence => Text::ExportPngSequence,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportQuality {
    Lower,
    #[default]
    Recommended,
    Higher,
}
impl ExportQuality {
    pub(super) fn label(self) -> Text {
        match self {
            Self::Lower => Text::ExportBitRateLower,
            Self::Recommended => Text::ExportBitRateRecommended,
            Self::Higher => Text::ExportBitRateHigher,
        }
    }
    /// The Bit rate preset's encoder settings: the encoder quality scalar and the
    /// target rate in bits per second. A Custom rate (`OutputOptions::bit_rate`)
    /// replaces the rate and keeps the last preset's scalar. This is the one mapping;
    /// the command declaration and the CLI carry only `OutputOptions`.
    pub fn value(self) -> (u8, usize) {
        match self {
            Self::Lower => (70, 6_000_000),
            Self::Recommended => (85, 12_000_000),
            Self::Higher => (95, 24_000_000),
        }
    }
}
/// The files ticked beside the primary output (`ExportOptions::format`): the dialog's
/// Audio and GIF sections whenever they are not the one holding the primary, and Captions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlsoOutputs {
    /// A separate audio file in this audio format.
    pub audio: Option<ExportFormat>,
    /// A GIF at this size, the GIF section's own resolution.
    pub gif: Option<[u32; 2]>,
    /// A caption file in this format, written beside the saved media.
    pub captions: Option<captions::Format>,
}
/// Saved app choices. Consent, overwrite authorization and destination are transient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub quality: ExportQuality,
    pub include_audio: bool,
    pub output: OutputOptions,
    pub also: AlsoOutputs,
}
impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Mp4,
            quality: ExportQuality::Recommended,
            include_audio: true,
            output: OutputOptions::default(),
            also: AlsoOutputs::default(),
        }
    }
}
impl ExportOptions {
    /// One single-output choice per file kind this export writes: the primary output,
    /// then the Audio file, then the GIF. Each runs as its own engine export job; scene
    /// and range are shared, and the video-only fields stay with the primary output.
    pub fn outputs(self) -> Vec<Self> {
        let primary = Self {
            also: AlsoOutputs::default(),
            ..self
        };
        let mut all = vec![primary];
        if let Some(format) = self.also.audio
            && !self.format.audio_only()
            && !self.format.is_sequence()
        {
            all.push(Self {
                format,
                include_audio: true,
                output: OutputOptions {
                    resolution: None,
                    fps: None,
                    bit_rate: None,
                    codec: render::VideoCodec::Automatic,
                    profile: media::H264Profile::Automatic,
                    ..self.output
                },
                ..primary
            });
        }
        if let Some(size) = self.also.gif
            && !self.format.is_gif()
        {
            all.push(Self {
                format: ExportFormat::Gif,
                output: OutputOptions {
                    resolution: Some(size),
                    // The Frame rate row is the video's; an audio-only primary has none.
                    fps: if self.format.audio_only() {
                        None
                    } else {
                        self.output.fps
                    },
                    bit_rate: None,
                    codec: render::VideoCodec::Automatic,
                    profile: media::H264Profile::Automatic,
                    ..self.output
                },
                ..primary
            });
        }
        all
    }
    pub fn settings(self, project: &project::Project) -> render::ExportSettings {
        let mut settings = render::ExportSettings::from_project(project);
        settings.container = self.format.container();
        settings.audio_only = self.format.audio_only();
        settings.image_sequence = self.format.is_sequence();
        (settings.quality, settings.bit_rate) = self.quality.value();
        settings.include_audio = self.include_audio;
        // A chosen quality must be applied, rather than bypassed by Auto copy.
        settings.mode = render::ExportMode::Encode;
        settings
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportPhase {
    Settings,
    Running,
    Ready,
    Failed,
}
#[derive(Clone)]
pub struct ExportInspection {
    pub open: bool,
    pub phase: ExportPhase,
    pub job: Option<JobSnapshot>,
    pub unavailable: Vec<render::Unavailable>,
    pub consent: bool,
    pub empty: bool,
    pub error: Option<Arc<Error>>,
    pub finished: Option<PathBuf>,
    pub diagnostics: Option<String>,
    /// Every file published since the export started, in save order.
    pub saved: Vec<PathBuf>,
    pub attempt: Option<ExportOptions>,
    /// The pixel size of the sized cover render when it is the image on screen.
    pub cover: Option<[usize; 2]>,
}
#[derive(Clone, Debug)]
pub struct SaveRequest {
    pub name: String,
}
/// One pending export plus the word-warning refusal its dialog reports.
type WordWarnings = (
    Arc<project::Project>,
    ExportOptions,
    usize,
    u64,
    std::result::Result<project::CaptionWordWarnings, Arc<Error>>,
);

/// One rendered file, or image-sequence directory, and what it holds.
struct Rendered {
    path: PathBuf,
    format: ExportFormat,
    /// The scene an all-scenes export rendered it for.
    scene: Option<project::SceneId>,
}
/// Where the files of one export are saved: beside the file the user picked, under derived
/// names, plus the caption file when Captions is ticked.
struct Group {
    dir: PathBuf,
    base: String,
    caption: Option<(PathBuf, CaptionOutput)>,
}
/// One engine export job: a single output kind, ready to submit.
struct Queued {
    options: ExportOptions,
    paths: Vec<PathBuf>,
    settings: render::ExportSettings,
    key: CacheKey,
}

pub struct ExportDialog {
    pub(crate) open: bool,
    consent: bool,
    unavailable: Vec<render::Unavailable>,
    word_warnings: Option<WordWarnings>,
    refused: Option<(Arc<project::Project>, bool)>,
    empty: bool,
    error: Option<Arc<Error>>,
    render_state: Arc<crate::services::playback::RenderState>,
    runner: JobRunner,
    sources: Arc<dyn crate::services::playback::Sources>,
    job: Option<(JobId, CancelToken)>,
    snapshot: Option<JobSnapshot>,
    cancelling: bool,
    temp: Option<Rendered>,
    /// Outputs after the first, of every job of this export, saved in order.
    scene_outputs: std::collections::VecDeque<Rendered>,
    /// The jobs still to run after the current one, one per further output kind.
    waiting: std::collections::VecDeque<Queued>,
    job_total: usize,
    job_done: usize,
    attempt_project: Option<Arc<project::Project>>,
    attempt_encoder: EncodePreference,
    /// The desktop effect adapter the next start lends to its render settings.
    effects: Option<Arc<dyn render::effects::EffectCompositor>>,
    /// The adapter the attempt was started with; Retry reruns with it.
    attempt_effects: Option<Arc<dyn render::effects::EffectCompositor>>,
    pub(crate) selection_range: Option<time::TimeRange>,
    finished: Option<Rendered>,
    pending_save: bool,
    /// The render finished since the last `take_completed`; the Edit settings' export sound.
    completed: bool,
    attempt: Option<ExportOptions>,
    diagnostics: Option<String>,
    saved: Vec<PathBuf>,
    /// Where the rest of the export goes once the first file's place is picked.
    group: Option<Group>,
    /// Replace files that already exist at the derived names of the group.
    replace: bool,
    protected: Vec<PathBuf>,
    caption_writes: Vec<(PathBuf, CaptionOutput)>,
    publishing: Option<PathBuf>,
    input: Option<CacheKey>,
    publication_fs: Arc<dyn project::atomic::AtomicFs + Send + Sync>,
    /// The project cover as last uploaded, to upload again only when it changes.
    cover: super::cover::Held,
}
impl ExportDialog {
    /// One export job: the caller's byte budget and the shared render state, no
    /// editor. The dialog owns its runner and temporary output.
    pub fn new(
        budget: ByteBudget,
        render_state: Arc<crate::services::playback::RenderState>,
    ) -> Result<Self> {
        Ok(Self {
            open: false,
            consent: false,
            unavailable: Vec::new(),
            word_warnings: None,
            refused: None,
            empty: false,
            error: None,
            render_state,
            runner: JobRunner::new(
                RunnerConfig {
                    concurrency: [1, 1, 1],
                    queue: 1,
                    retained: 1,
                    network_attempts: 1,
                    backoff: Duration::from_secs(1),
                    progress_interval: Duration::from_millis(20),
                },
                budget,
                Arc::new(SystemClock::default()),
            )?,
            sources: Arc::new(crate::services::playback::Files),
            job: None,
            snapshot: None,
            cancelling: false,
            temp: None,
            scene_outputs: Default::default(),
            waiting: Default::default(),
            job_total: 0,
            job_done: 0,
            attempt_project: None,
            attempt_encoder: EncodePreference::default(),
            effects: None,
            attempt_effects: None,
            selection_range: None,
            finished: None,
            pending_save: false,
            completed: false,
            attempt: None,
            diagnostics: None,
            saved: Vec::new(),
            group: None,
            replace: false,
            protected: Vec::new(),
            caption_writes: Vec::new(),
            publishing: None,
            input: None,
            publication_fs: Arc::new(project::atomic::Fs),
            cover: Default::default(),
        })
    }
    /// Inject only the render source boundary; scheduling stays with engine.
    pub fn set_sources(
        &mut self,
        sources: Arc<dyn crate::services::playback::Sources>,
    ) -> Result<()> {
        if self.active() {
            return Err(Error::Invalid("export in progress"));
        }
        self.sources = sources;
        Ok(())
    }
    /// Lend the desktop effect adapter to the next start. A running attempt keeps the
    /// adapter it was started with.
    pub fn set_effects(&mut self, effects: Arc<dyn render::effects::EffectCompositor>) {
        self.effects = Some(effects);
    }
    /// Inject the existing storage boundary; IO still runs on the engine worker.
    pub fn set_publication_fs(
        &mut self,
        fs: Arc<dyn project::atomic::AtomicFs + Send + Sync>,
    ) -> Result<()> {
        if self.active() {
            return Err(Error::Invalid("export in progress"));
        }
        self.publication_fs = fs;
        Ok(())
    }
    pub fn phase(&self) -> ExportPhase {
        if self.active() {
            ExportPhase::Running
        } else if self.error.is_some()
            || self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.state == JobState::Failed)
        {
            ExportPhase::Failed
        } else if self.finished.is_some() {
            ExportPhase::Ready
        } else {
            ExportPhase::Settings
        }
    }
    pub fn inspect(&self) -> ExportInspection {
        ExportInspection {
            open: self.open,
            phase: self.phase(),
            job: self.snapshot.clone(),
            unavailable: self.unavailable.clone(),
            consent: self.consent,
            empty: self.empty,
            error: self.error.clone(),
            finished: self.finished.as_ref().map(|file| file.path.clone()),
            diagnostics: self.diagnostics.clone(),
            saved: self.saved.clone(),
            attempt: self.attempt,
            cover: self.cover.sharp_size(),
        }
    }
    pub fn active(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|s| {
            !matches!(
                s.state,
                JobState::Completed | JobState::Cancelled | JobState::Failed
            )
        })
    }
    pub fn show(&mut self, project: &project::Project) {
        if !self.open {
            self.consent = false;
            if self.finished.is_none() && !self.active() {
                self.error = None;
                self.diagnostics = None;
                self.snapshot = None;
                self.job = None;
                self.unavailable.clear();
                self.refused = None;
                self.attempt_project = None;
            }
        }
        self.open = true;
        self.empty = project.duration() == time::Tick::ZERO;
    }
    pub fn cancel(&mut self) {
        if let Some((_, token)) = &self.job {
            token.cancel();
            self.cancelling = self.active();
        }
    }
    /// Whether a render finished since the last call; reading clears it.
    pub fn take_completed(&mut self) -> bool {
        std::mem::take(&mut self.completed)
    }
    pub fn tick(
        &mut self,
        _project: &Arc<project::Project>,
        _options: ExportOptions,
    ) -> Result<()> {
        self.runner.tick()?;
        self.runner.drain_events();
        if let Some((id, _)) = self.job {
            self.snapshot = self.runner.snapshot(id);
        }
        let state = self.snapshot.as_ref().map(|snapshot| snapshot.state);
        let failure = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.error.clone());
        match state {
            Some(JobState::Completed) => {
                let cancelled = std::mem::take(&mut self.cancelling);
                if let Some(path) = self.publishing.take() {
                    if let Some(finished) = self.finished.take() {
                        remove_output(&finished.path);
                    }
                    self.saved.push(path);
                    self.finished = self.scene_outputs.pop_front();
                    self.pending_save = false;
                    self.open = self.finished.is_some();
                    self.snapshot = None;
                    self.job = None;
                    self.diagnostics = None;
                    // The place was picked once: the next file goes beside the first.
                    let next = match (&self.finished, &self.group) {
                        (Some(file), Some(group)) => Some(self.derived(group, file)),
                        _ => None,
                    };
                    if let Some(target) = next {
                        let protected = self.protected.clone();
                        return self.publish(target, protected);
                    }
                    if self.finished.is_none()
                        && let Some(group) = self.group.take()
                        && let Some((path, output)) = group.caption
                    {
                        self.saved.push(path.clone());
                        self.caption_writes.push((path, output));
                    }
                    self.pending_save = self.finished.is_some();
                    return Ok(());
                }
                // One job renders one output kind; the next starts when this one is done,
                // unless the user cancelled meanwhile.
                if !self.waiting.is_empty() {
                    if cancelled {
                        self.abandon();
                        return Ok(());
                    }
                    return self.start_next();
                }
                if self.finished.is_none() {
                    self.finished = self.temp.take();
                    self.pending_save = self.finished.is_some();
                    self.completed |= self.finished.is_some();
                }
            }
            Some(JobState::Failed) => {
                self.waiting.clear();
                if self.publishing.is_none() {
                    self.discard_scene_outputs();
                }
                if self.publishing.is_none()
                    && let Some(file) = self.temp.take()
                {
                    remove_output(&file.path);
                }
                self.cancelling = false;
                let unavailable = failure.as_deref().and_then(|error| {
                    let raw = error as &(dyn std::error::Error + 'static);
                    match raw.downcast_ref::<engine::control::Error>() {
                        Some(engine::control::Error::Render(render::Error::Unavailable(items)))
                        | Some(engine::control::Error::Runtime(engine::RuntimeError::Render(
                            render::Error::Unavailable(items),
                        ))) => Some(items),
                        _ => match raw.downcast_ref::<engine::RuntimeError>() {
                            Some(engine::RuntimeError::Render(render::Error::Unavailable(
                                items,
                            ))) => Some(items),
                            _ => None,
                        },
                    }
                });
                if let Some(items) = unavailable
                    && self.unavailable != *items
                {
                    self.consent = false;
                    self.unavailable = items.clone();
                }
                self.diagnostics = failure
                    .as_ref()
                    .map(|error| diagnostics(error.as_ref(), &self.unavailable));
            }
            Some(JobState::Cancelled) => {
                self.waiting.clear();
                if self.publishing.is_none() {
                    self.discard_scene_outputs();
                }
                if let Some(temp) = self.temp.take() {
                    remove_output(&temp.path);
                }
                self.cancelling = false;
                self.snapshot = None;
                self.job = None;
                if self.publishing.take().is_none() {
                    self.finished = None;
                }
                self.pending_save = false;
                self.diagnostics = None;
                self.error = None;
            }
            _ => {}
        }
        Ok(())
    }
    /// A cancel that arrived while a later job was still waiting: drop what rendered.
    fn abandon(&mut self) {
        self.waiting.clear();
        self.discard_scene_outputs();
        if let Some(temp) = self.temp.take() {
            remove_output(&temp.path);
        }
        self.cancelling = false;
        self.snapshot = None;
        self.job = None;
        self.finished = None;
        self.pending_save = false;
        self.diagnostics = None;
        self.error = None;
    }
    /// Submits the next output kind's job; a refusal ends the export and drops what rendered.
    fn start_next(&mut self) -> Result<()> {
        let project = self
            .attempt_project
            .clone()
            .ok_or(Error::Invalid("no export snapshot"))?;
        let Some(next) = self.waiting.pop_front() else {
            return Ok(());
        };
        self.job_done += 1;
        if let Err(error) = self.submit(&project, next, false) {
            self.waiting.clear();
            self.discard_scene_outputs();
            if let Some(temp) = self.temp.take() {
                remove_output(&temp.path);
            }
            return Err(error);
        }
        Ok(())
    }
    /// One output kind's job: its settings, scene files and cache key, built before any
    /// job of the export is submitted so a refusal leaves nothing running.
    fn prepare(
        &self,
        project: &Arc<project::Project>,
        options: ExportOptions,
        encoder: EncodePreference,
        effects: Option<&Arc<dyn render::effects::EffectCompositor>>,
        ordinal: usize,
    ) -> Result<Queued> {
        let path = temp_output(options.format, ordinal);
        let paths = options.output.paths(project, &path)?;
        let mut settings = options.settings(project);
        settings.encoder = encoder;
        settings.render = self.render_state.settings(project)?;
        settings.render.effects = effects.cloned();
        options.output.apply(&mut settings)?;
        settings.overwrite = project::atomic::Overwrite::Replace;
        let same = self
            .refused
            .as_ref()
            .is_some_and(|(p, a)| Arc::ptr_eq(p, project) && *a == options.include_audio);
        settings.allow_degraded = self.consent && same && !self.unavailable.is_empty();
        if settings.allow_degraded {
            settings.degraded_consent = Some(self.unavailable.clone());
        }
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::Preview,
            source: [0; 32],
            provider: "desktop",
            model: "export",
            version: "1",
            settings: &serde_json::to_vec(&options)?,
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        Ok(Queued {
            options,
            paths,
            settings,
            key,
        })
    }
    /// Submits one prepared job. The first job of an export replaces the previous
    /// export's files; later ones add their files after it.
    fn submit(
        &mut self,
        project: &Arc<project::Project>,
        queued: Queued,
        first: bool,
    ) -> Result<()> {
        let Queued {
            options,
            paths,
            settings,
            key,
        } = queued;
        let scenes: Vec<_> = options
            .output
            .ranges(project)?
            .into_iter()
            .map(|(id, _)| (options.output.scene == SceneChoice::All).then_some(id))
            .collect();
        let sources = self.sources.clone();
        let job = engine::submit_scene_exports_with_sources(
            &mut self.runner,
            key,
            project.clone(),
            options.output,
            settings,
            paths.clone(),
            move || sources.create(),
        )?;
        self.snapshot = self.runner.snapshot(job.0);
        self.job = Some(job);
        self.input = Some(key);
        let mut files = paths.into_iter().zip(scenes).map(|(path, scene)| Rendered {
            path,
            format: options.format,
            scene,
        });
        if first {
            self.discard_finished();
            self.temp = files.next();
            self.scene_outputs = files.collect();
        } else {
            self.scene_outputs.extend(files);
        }
        Ok(())
    }
    /// Starts the export: one engine job per ticked output kind, run one after the other
    /// under one progress. Every job is prepared first, so a refusal starts none.
    pub fn start(
        &mut self,
        project: Arc<project::Project>,
        options: ExportOptions,
        encoder: EncodePreference,
    ) -> Result<()> {
        self.begin(project, options, encoder, self.effects.clone())
    }
    /// One attempt over an immutable snapshot: project, choices, encoder and adapter.
    fn begin(
        &mut self,
        project: Arc<project::Project>,
        options: ExportOptions,
        encoder: EncodePreference,
        effects: Option<Arc<dyn render::effects::EffectCompositor>>,
    ) -> Result<()> {
        if self.active() {
            return Err(Error::Invalid("export in progress"));
        }
        self.empty = match options.output.scene {
            SceneChoice::Selected { scene } => {
                project
                    .data()
                    .scenes
                    .get(scene)
                    .ok_or(Error::Invalid("scene not found"))?
                    .duration()
                    == time::Tick::ZERO
            }
            _ => project.duration() == time::Tick::ZERO,
        };
        if self.empty {
            return Err(Error::EmptyTimeline);
        }
        let mut queue = options
            .outputs()
            .into_iter()
            .enumerate()
            .map(|(ordinal, output)| {
                self.prepare(&project, output, encoder, effects.as_ref(), ordinal)
            })
            .collect::<Result<std::collections::VecDeque<_>>>()?;
        let first = queue
            .pop_front()
            .ok_or(Error::Invalid("no export output"))?;
        let strict = !first.settings.allow_degraded;
        self.submit(&project, first, true)?;
        if strict {
            self.consent = false;
            self.unavailable.clear();
        }
        self.job_total = queue.len() + 1;
        self.job_done = 0;
        self.waiting = queue;
        self.cancelling = false;
        self.error = None;
        self.attempt_project = Some(project.clone());
        self.attempt_encoder = encoder;
        self.attempt_effects = effects;
        self.saved.clear();
        self.group = None;
        self.caption_writes.clear();
        self.finished = None;
        self.pending_save = false;
        self.diagnostics = None;
        self.attempt = Some(options);
        self.publishing = None;
        self.refused = Some((project, options.include_audio));
        Ok(())
    }
    pub fn retry(&mut self, project: Arc<project::Project>) -> Result<()> {
        if self.phase() != ExportPhase::Failed {
            return Err(Error::Invalid("no failed export to retry"));
        }
        if self.finished.is_some() {
            // A save failure retries destination selection, preserving rendered bytes.
            self.snapshot = None;
            self.job = None;
            self.publishing = None;
            self.diagnostics = None;
            self.pending_save = true;
            self.error = None;
            return Ok(());
        }
        let options = self
            .attempt
            .ok_or(Error::Invalid("no export settings to retry"))?;
        self.begin(
            self.attempt_project.clone().unwrap_or(project),
            options,
            self.attempt_encoder,
            self.attempt_effects.clone(),
        )
    }
    pub fn take_save_request(
        &mut self,
        project: &project::Project,
        options: ExportOptions,
    ) -> Option<SaveRequest> {
        if !self.pending_save {
            return None;
        }
        self.pending_save = false;
        Some(SaveRequest {
            name: self.save_name(project, options),
        })
    }
    /// The default name of the file the next save writes: the project's name, the scene of
    /// an all-scenes export, and `-audio` for an audio file saved beside a video.
    pub fn save_name(&self, project: &project::Project, options: ExportOptions) -> String {
        let options = self.attempt.unwrap_or(options);
        let project = self.attempt_project.as_deref().unwrap_or(project);
        let (format, scene) = self
            .finished
            .as_ref()
            .map_or((options.format, None), |file| (file.format, file.scene));
        let mut name = project.data().name.as_str().to_owned();
        if let Some(scene) = scene {
            name.push_str(&format!("-scene-{}", scene.0));
        }
        if format.audio_only() && !options.format.audio_only() {
            name.push_str("-audio");
        }
        format.file_name(&name)
    }
    /// The format of the file the next save writes, once one has rendered.
    pub fn save_format(&self) -> Option<ExportFormat> {
        self.finished.as_ref().map(|file| file.format)
    }
    /// The place of the export's first file was picked: refuse up front when a file the rest
    /// would write already exists (unless Replace is ticked), then publish it. The other
    /// files follow from `tick`, beside it.
    pub fn save_to(&mut self, path: PathBuf, protected: Vec<PathBuf>) -> Result<()> {
        if self.active() {
            return Err(Error::Invalid("export in progress"));
        }
        let scene = self
            .finished
            .as_ref()
            .ok_or(Error::Invalid("no finished export"))?
            .scene;
        if self.saved.is_empty() {
            let group = self.group_for(&path, scene)?;
            let mut targets: Vec<PathBuf> = self
                .scene_outputs
                .iter()
                .map(|file| self.derived(&group, file))
                .collect();
            targets.extend(group.caption.iter().map(|(path, _)| path.clone()));
            if !self.replace && targets.iter().any(|target| target.exists()) {
                return Err(Error::Invalid("export target exists"));
            }
            self.group = Some(group);
            self.protected = protected.clone();
        }
        self.publish(path, protected)
    }
    /// The group of files saved beside the one the user picked.
    fn group_for(&self, path: &std::path::Path, scene: Option<project::SceneId>) -> Result<Group> {
        let dir = path.parent().map(PathBuf::from).unwrap_or_default();
        let mut base = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or(Error::Invalid("export name"))?
            .to_owned();
        // The picked name already carries its own scene.
        if let Some(scene) = scene
            && let Some(stripped) = base.strip_suffix(&format!("-scene-{}", scene.0))
        {
            base = stripped.to_owned();
        }
        let caption = self
            .attempt
            .and_then(|attempt| attempt.also.captions)
            .filter(|_| {
                self.attempt_project
                    .as_deref()
                    .is_some_and(super::caption_outputs::captioned)
            })
            .map(|format| {
                let output = CaptionOutput::File(format);
                (dir.join(format!("{base}.{}", output.extension())), output)
            });
        Ok(Group { dir, base, caption })
    }
    /// Where `file` goes beside the first file: the scene and `-audio` as in the default names.
    fn derived(&self, group: &Group, file: &Rendered) -> PathBuf {
        let mut name = group.base.clone();
        if let Some(scene) = file.scene {
            name.push_str(&format!("-scene-{}", scene.0));
        }
        if file.format.audio_only() && !self.attempt.is_some_and(|a| a.format.audio_only()) {
            name.push_str("-audio");
        }
        group.dir.join(file.format.file_name(&name))
    }
    fn publish(&mut self, path: PathBuf, protected: Vec<PathBuf>) -> Result<()> {
        let finished = self
            .finished
            .as_ref()
            .ok_or(Error::Invalid("no finished export"))?;
        let source = finished.path.clone();
        let sequence = finished.format.is_sequence();
        let project = self
            .refused
            .as_ref()
            .ok_or(Error::Invalid("no export snapshot"))?
            .0
            .clone();
        let key = self.input.ok_or(Error::Invalid("no export job"))?;
        let job = if sequence {
            engine::submit_sequence_publication(
                &mut self.runner,
                key,
                project,
                source,
                path.clone(),
                protected,
                self.publication_fs.clone(),
            )?
        } else {
            engine::submit_export_publication(
                &mut self.runner,
                key,
                project,
                source,
                path.clone(),
                protected,
                self.publication_fs.clone(),
            )?
        };
        self.snapshot = self.runner.snapshot(job.0);
        self.job = Some(job);
        self.publishing = Some(path);
        self.pending_save = false;
        self.diagnostics = None;
        self.error = None;
        Ok(())
    }
    /// The caption files the window must write now that the media is saved.
    pub fn take_caption_writes(&mut self) -> Vec<(PathBuf, CaptionOutput)> {
        std::mem::take(&mut self.caption_writes)
    }
    pub fn save_cancelled(&mut self) {
        self.pending_save = false;
    }
    /// Every file this export saved, once the last one is: the caller toasts them together.
    pub fn take_saved(&mut self) -> Option<Vec<PathBuf>> {
        if self.finished.is_some() || self.saved.is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.saved))
    }
    pub fn discard_finished(&mut self) {
        self.group = None;
        self.discard_scene_outputs();
        if let Some(file) = self.finished.take() {
            remove_output(&file.path);
        }
        self.pending_save = false;
    }
    fn discard_scene_outputs(&mut self) {
        for file in self.scene_outputs.drain(..) {
            remove_output(&file.path);
        }
    }
}

/// The unique name of one job's staged output; `ordinal` separates the jobs of one export.
fn temp_output(format: ExportFormat, ordinal: usize) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let name = format!("vibecut-export-{}-{nanos}-{ordinal}", std::process::id());
    std::env::temp_dir().join(if format.is_sequence() {
        name
    } else {
        format!("{name}.{}", format.extension())
    })
}
/// Removes a finished or abandoned temporary output: a file, or an image sequence's directory.
fn remove_output(path: &std::path::Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}
fn diagnostics(
    error: &(dyn std::error::Error + 'static),
    unavailable: &[render::Unavailable],
) -> String {
    let mut text = format!("{error:?}");
    for item in unavailable {
        text.push_str(&format!(
            "\nasset {} item {} range {}..{} {:?}",
            item.asset.0,
            item.item.0,
            item.range.start().0,
            item.range.end().0,
            item.reason
        ));
    }
    text
}
pub use engine::{default_file_name, sanitize_file_name};

pub(crate) enum DialogAction {
    Retry,
    Save,
    Start,
    Caption(crate::services::lifecycle::CaptionOutput),
    /// The Edit cover chip: the frame at the playhead becomes the project cover.
    EditCover,
}
/// The export window: 720 x 663 as in the reference capture (row 12 of the 2026-11-14b round).
const DIALOG_SIZE: [f32; 2] = [720.0, 663.0];
pub(crate) struct DialogUi {
    pub widgets: Vec<(&'static str, egui::Response)>,
    pub action: Option<DialogAction>,
}
impl ExportDialog {
    pub(crate) fn ui(
        &mut self,
        ctx: &egui::Context,
        locale: Locale,
        dark: bool,
        options: &mut ExportOptions,
        project: &Arc<project::Project>,
        cover: super::cover::Cover<'_>,
    ) -> DialogUi {
        let mut result = DialogUi {
            widgets: Vec::new(),
            action: None,
        };
        if !self.open {
            return result;
        }
        let used = self
            .runner
            .memory_budget()
            .snapshot()
            .map_or(u64::MAX, |snapshot| snapshot.limited_used());
        if self
            .word_warnings
            .as_ref()
            .is_none_or(|(prior, settings, fonts, prior_used, result)| {
                !Arc::ptr_eq(prior, project)
                    || *settings != *options
                    || *fonts != text::generation()
                    || (result.is_err() && *prior_used != used)
            })
        {
            let warnings = self
                .render_state
                .settings(project)
                .and_then(|render| {
                    let mut settings = options.settings(project);
                    settings.render = render;
                    options
                        .output
                        .caption_word_warnings(
                            project,
                            &settings,
                            &engine::ByteBudgetHooks::new(self.runner.memory_budget(), None),
                        )
                        .map_err(Error::from)
                })
                .map_err(Arc::new);
            self.word_warnings = Some((
                project.clone(),
                *options,
                text::generation(),
                used,
                warnings,
            ));
        }
        let tr = |k| i18n::text(locale, k);
        let active = self.active();
        let frame = crate::kit::dialog_frame(ctx);
        let frame_margin = frame.total_margin();
        let modal = egui::Modal::new(egui::Id::unique("export.dialog"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.style_mut().override_font_id = Some(egui::FontId::proportional(14.0));
                ui.set_width(
                    DIALOG_SIZE[0].min(ctx.content_rect().width() - 48.0) - frame_margin.sum().x,
                );
                let dialog_height = DIALOG_SIZE[1].min(ctx.content_rect().height() - 48.0);
                ui.set_height(dialog_height - frame_margin.sum().y);
                crate::kit::dialog_title(ui, tr(Text::ExportTitle));
                ui.add_space(12.0);
                // The footer is laid out first from the bottom, so the settings scroll inside
                // whatever height remains and never run under it.
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    // A plain horizontal row in a bottom-up column takes the column's full
                    // height; the footer row is exactly one button tall.
                    let footer = egui::vec2(ui.available_width(), theme::size::DIALOG_BUTTON_HEIGHT);
                    let row = egui::Layout::left_to_right(egui::Align::Center);
                    ui.allocate_ui_with_layout(footer, row, |ui| {
                        let duration = options.output.ranges(project).map_or(0.0, |scenes| {
                            scenes
                                .iter()
                                .map(|(_, range)| range.duration().seconds())
                                .sum()
                        });
                        // The files of one export add up: one estimate per output kind.
                        let megabytes: f64 = options
                            .outputs()
                            .iter()
                            .map(|output| {
                                super::settings::estimated_megabytes(output, project, duration)
                            })
                            .sum();
                        ui.label(format!(
                            "{} | {} {} MB",
                            i18n::export_duration(locale, duration),
                            tr(Text::ExportEstimatedSize),
                            i18n::decimal(locale, megabytes, 1)
                        ));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| match self.phase() {
                                ExportPhase::Running => {
                                    let r = ui
                                        .add_enabled_ui(!self.cancelling, |ui| {
                                            crate::kit::dialog_button(
                                                ui,
                                                "export.cancel",
                                                tr(Text::Cancel),
                                                false,
                                            )
                                        })
                                        .inner;
                                    if r.clicked() {
                                        self.cancel();
                                    }
                                    result.widgets.push(("export.cancel", r));
                                }
                                ExportPhase::Failed => {
                                    let r = crate::kit::dialog_button(
                                        ui,
                                        "export.retry",
                                        tr(Text::ExportRetry),
                                        false,
                                    );
                                    if r.clicked() {
                                        result.action = Some(DialogAction::Retry);
                                    }
                                    result.widgets.push(("export.retry", r));
                                    let r = crate::kit::dialog_button(
                                        ui,
                                        "export.copy",
                                        tr(Text::ExportCopy),
                                        false,
                                    );
                                    if r.clicked()
                                        && let Some(text) = &self.diagnostics
                                    {
                                        ui.ctx().copy_text(text.clone());
                                    }
                                    result.widgets.push(("export.copy", r));
                                    let r = crate::kit::dialog_button(
                                        ui,
                                        "export.close",
                                        tr(Text::CloseScenes),
                                        false,
                                    );
                                    if r.clicked() {
                                        self.open = false;
                                    }
                                    result.widgets.push(("export.close", r));
                                }
                                ExportPhase::Ready => {
                                    let r = crate::kit::dialog_button(
                                        ui,
                                        "export.save",
                                        tr(Text::Save),
                                        false,
                                    );
                                    if r.clicked() {
                                        result.action = Some(DialogAction::Save);
                                    }
                                    result.widgets.push(("export.save", r));
                                    let r = crate::kit::dialog_button(
                                        ui,
                                        "export.close",
                                        tr(Text::Cancel),
                                        false,
                                    );
                                    if r.clicked() {
                                        self.discard_finished();
                                        self.open = false;
                                    }
                                    result.widgets.push(("export.close", r));
                                }
                                ExportPhase::Settings => {
                                    // Added first, so it sits rightmost: Cancel, then Export.
                                    let r = ui
                                        .add_enabled_ui(settings::valid(options, project), |ui| {
                                            crate::kit::dialog_button(
                                                ui,
                                                "export.run",
                                                tr(Text::Export),
                                                true,
                                            )
                                        })
                                        .inner;
                                    if r.clicked() {
                                        result.action = Some(DialogAction::Start);
                                    }
                                    result.widgets.push(("export.run", r));
                                    let r = crate::kit::dialog_button(
                                        ui,
                                        "export.close",
                                        tr(Text::Cancel),
                                        false,
                                    );
                                    if r.clicked() {
                                        self.open = false;
                                    }
                                    result.widgets.push(("export.close", r));
                                }
                            },
                        );
                    });
                    // A top-down child of a bottom-up column starts at the column top, so the
                    // status lines get last pass's measured height and re-run once if it changed.
                    let status_id = ui.scope_id().with("export.status.height");
                    let status_height =
                        ui.ctx().data(|data| data.get_temp::<f32>(status_id)).unwrap_or(0.0);
                    let status_size = egui::vec2(ui.available_width(), status_height);
                    let status_layout = egui::Layout::top_down(egui::Align::Min);
                    let status = ui.allocate_ui_with_layout(status_size, status_layout, |ui| {
                        self.empty = options.output.scene == SceneChoice::Active
                            && project.duration() == time::Tick::ZERO;
                        if self.empty {
                            ui.colored_label(theme::error_text(dark), tr(Text::ExportEmpty));
                        }
                        match self.phase() {
                            ExportPhase::Running => {
                                if let Some(snapshot) = &self.snapshot {
                                    let total = self.job_total.max(1);
                                    let number = |count: usize| i18n::number(locale, count as u64);
                                    let label = if self.cancelling {
                                        tr(Text::ExportCancelling).to_owned()
                                    } else if total > 1 {
                                        i18n::format(
                                            locale,
                                            Text::ExportProgressFiles,
                                            &[
                                                ("n", &number(self.job_done + 1)),
                                                ("total", &number(total)),
                                            ],
                                        )
                                    } else {
                                        tr(Text::ExportProgress).to_owned()
                                    };
                                    ui.label(label);
                                    // One progress over every job of the export.
                                    let mut overall = snapshot.clone();
                                    if total > 1 && self.publishing.is_none() {
                                        overall.progress = (self.job_done as f32
                                            + snapshot.progress.clamp(0.0, 1.0))
                                            / total as f32;
                                    }
                                    job_progress(ui, "export.progress", &overall, locale);
                                }
                            }
                            ExportPhase::Failed => {
                                let label = if matches!(
                                    self.error.as_deref(),
                                    Some(Error::Invalid("export target exists"))
                                ) {
                                    Text::ExportTargetExists
                                } else {
                                    self.snapshot
                                        .as_ref()
                                        .and_then(|snapshot| snapshot.error.as_deref())
                                        .map_or(Text::ExportFailure, export_failure)
                                };
                                ui.colored_label(theme::error_text(dark), tr(label));
                                self.replace_choice(ui, locale, &mut result);
                            }
                            ExportPhase::Ready | ExportPhase::Settings => {
                                if self.error.is_some() {
                                    ui.colored_label(theme::error_text(dark), tr(Text::ExportFailure));
                                }
                                self.replace_choice(ui, locale, &mut result);
                            }
                        }
                    });
                    let measured = status.response.rect.height();
                    if (measured - status_height).abs() > 0.5 {
                        ui.ctx().data_mut(|data| data.insert_temp(status_id, measured));
                        ui.ctx().request_discard("export status height");
                    }
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(16.0);
                    // A bottom-up cursor has no top edge, so the form gets an explicit rect from
                    // the column top down to the separator.
                    let body = egui::Rect::from_min_max(
                        ui.max_rect().min,
                        egui::pos2(ui.max_rect().right(), ui.cursor().max.y),
                    );
                    let builder = egui::UiBuilder::new()
                        .max_rect(body)
                        .layout(egui::Layout::top_down(egui::Align::Min));
                    ui.scope_builder(builder, |ui| {
                        let body_height = ui.available_height().max(0.0);
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let gap = 20.0;
                            let settings_width = ui.available_width() - 227.0 - gap;
                            ui.allocate_ui_with_layout(
                                egui::vec2(227.0, body_height),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    super::cover::show(
                                        ui,
                                        &mut self.cover,
                                        cover,
                                        super::cover::View {
                                            locale,
                                            dark,
                                            aspect: project.data().canvas.height as f32
                                                / project.data().canvas.width as f32,
                                            height: body_height,
                                            empty: self.empty,
                                            busy: active,
                                            budget: self.runner.memory_budget(),
                                        },
                                        &mut result,
                                    );
                                },
                            );
                            ui.add_space(gap);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.set_width(settings_width);
                                ui.set_min_height(body_height);
                                egui::ScrollArea::vertical()
                                    .id_salt("export.settings")
                                    .max_height(body_height)
                                    .show(ui, |ui| {
                                        ui.add_enabled_ui(!active, |ui| {
                                            if !self.unavailable.is_empty() {
                                                ui.colored_label(
                                                    theme::warning(dark),
                                                    tr(Text::ExportBlocked),
                                                );
                                                unavailable_details(ui, project, &self.unavailable, locale);
                                                let r = ui
                                                    .checkbox(&mut self.consent, tr(Text::ExportDegraded));
                                                result.widgets.push(("export.consent", r));
                                                ui.add_space(14.0);
                                            }
                                            if let Some((_,_,_,_,warnings)) = &self.word_warnings {
                                                match warnings {
                                                    Ok(warnings) => {
                                                        let reasons: std::collections::BTreeSet<_> = warnings.warnings.iter().map(|warning| warning.reason).collect();
                                                        for reason in reasons {
                                                            let key = match reason {
                                                                captions::WordHighlightReason::MissingTiming => Text::SubWordMissing,
                                                                captions::WordHighlightReason::SourceChanged => Text::SubWordChanged,
                                                                captions::WordHighlightReason::SourceNotDisplayed => Text::SubWordLanguage,
                                                                captions::WordHighlightReason::AmbiguousTiming => Text::SubWordAmbiguous,
                                                                captions::WordHighlightReason::ShapedBoundary => Text::SubWordShaped,
                                                            };
                                                            let response = ui.colored_label(theme::warning(dark),tr(key));
                                                            result.widgets.push(("export.word_highlight.warning",response));
                                                        }
                                                    }
                                                    Err(error) => { ui.label(tr(Text::SubWordUnavailable)).on_hover_text(error.to_string()); }
                                                }
                                            }
                                            settings::show(
                                                ui,
                                                locale,
                                                options,
                                                project,
                                                self.selection_range,
                                                &mut result,
                                            );
                                            ui.add_space(14.0);
                                            super::caption_outputs::show(
                                                ui,
                                                locale,
                                                project,
                                                options,
                                                &mut result,
                                            );
                                            ui.add_space(14.0);
                                        });
                                    });
                            });
                        });
                    });
                });
            });
        if modal.should_close() {
            if active {
                self.cancel();
            } else {
                self.discard_finished();
                self.open = false;
            }
        }
        result
    }
    /// One Replace choice for the whole group, offered while the first file awaits its place
    /// and the group holds more than that file.
    fn replace_choice(&mut self, ui: &mut egui::Ui, locale: Locale, result: &mut DialogUi) {
        let several = !self.scene_outputs.is_empty()
            || self
                .attempt
                .is_some_and(|attempt| attempt.also.captions.is_some());
        if self.finished.is_some() && self.saved.is_empty() && several {
            let r = ui.checkbox(&mut self.replace, i18n::text(locale, Text::ExportReplace));
            result.widgets.push(("export.replace", r));
        }
    }
    pub(crate) fn failed(&mut self, error: Error) {
        self.diagnostics = Some(format!("{error:?}"));
        self.error = Some(Arc::new(error));
    }
}

impl std::fmt::Debug for ExportInspection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportInspection")
            .field("open", &self.open)
            .field("phase", &self.phase)
            .field("state", &self.job.as_ref().map(|s| s.state))
            .field("unavailable", &self.unavailable)
            .field("consent", &self.consent)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}
fn unavailable_details(
    ui: &mut egui::Ui,
    project: &project::Project,
    unavailable: &[render::Unavailable],
    locale: Locale,
) {
    for item in unavailable {
        if let Some(name) = project
            .data()
            .assets
            .iter()
            .find(|asset| asset.id() == item.asset)
            .map(|asset| project.data().media_library.display_name(asset))
        {
            ui.add(egui::Label::new(name).truncate());
        }
        let reason = match item.reason {
            render::UnavailableReason::Missing => Text::SourceMissing,
            render::UnavailableReason::Changed => Text::SourceChanged,
            render::UnavailableReason::Metadata => Text::SourceMetadata,
            render::UnavailableReason::Video => Text::SourceVideo,
            render::UnavailableReason::Audio => Text::SourceAudio,
        };
        ui.label(format!(
            "{} {} · {} {} · {}",
            i18n::text(locale, Text::SourceAsset),
            item.asset.0,
            i18n::text(locale, Text::SourceItem),
            item.item.0,
            i18n::text(locale, reason)
        ));
        ui.label(i18n::export_range(
            locale,
            item.range.start().seconds(),
            item.range.end().seconds(),
        ));
    }
}

fn export_failure(error: &(dyn engine::Failure + 'static)) -> Text {
    use engine::{RuntimeError as B, control::Error as E};
    use render::Error as R;
    let cause: &(dyn std::error::Error + 'static) = error;
    let root = cause.downcast_ref::<E>();
    let runtime = match root {
        Some(E::Runtime(runtime)) => Some(runtime),
        _ => cause.downcast_ref::<B>(),
    };
    let rendered = match root {
        Some(E::Render(rendered)) => Some(rendered),
        _ => match runtime {
            Some(B::Render(rendered)) => Some(rendered),
            _ => None,
        },
    };
    match (root, runtime, rendered) {
        (_, Some(B::BudgetFull | B::ArtifactTooLarge), _)
        | (_, _, Some(R::Budget { .. } | R::Allocation(_))) => Text::ExportMemory,
        (_, _, Some(R::AudioOverload { .. })) => Text::ExportOverload,
        (_, _, Some(R::Unavailable(_))) => Text::ExportBlocked,
        (Some(E::Io(_)), _, _)
        | (_, Some(B::Io(_)), _)
        | (_, _, Some(R::Io(_) | R::Project(project::Error::Io(_)))) => Text::ExportStorage,
        (_, _, Some(R::Media(_) | R::Unsupported(_))) => Text::ExportCapability,
        (_, _, Some(R::Text(_) | R::Raster(_))) => Text::ExportTextFailure,
        _ => Text::ExportFailure,
    }
}

impl Drop for ExportDialog {
    fn drop(&mut self) {
        self.runner.shutdown();
        self.discard_finished();
        if let Some(file) = self.temp.take() {
            remove_output(&file.path);
        }
    }
}
