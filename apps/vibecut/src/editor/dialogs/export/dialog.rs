//! Presents one export job and its saved choices.
use super::settings;
use crate::i18n;
use crate::services::progress::job_progress;
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
            Self::Gif => media::Container::Gif,
            Self::PngSequence => media::Container::Mp4,
        }
    }
    pub fn audio_only(self) -> bool {
        matches!(self, Self::M4a | Self::AudioWebm)
    }
    pub(super) fn label(self) -> Text {
        match self {
            Self::Mp4 => Text::ExportMp4,
            Self::Mov => Text::ExportMov,
            Self::Webm => Text::ExportWebm,
            Self::M4a => Text::ExportAudioM4a,
            Self::AudioWebm => Text::ExportAudioWebm,
            Self::Gif => Text::ExportGif,
            Self::PngSequence => Text::ExportPngSequence,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportQuality {
    Low,
    Medium,
    #[default]
    High,
    VeryHigh,
}
impl ExportQuality {
    pub(super) fn label(self) -> Text {
        match self {
            Self::Low => Text::QualityLow,
            Self::Medium => Text::QualityMedium,
            Self::High => Text::QualityHigh,
            Self::VeryHigh => Text::QualityVeryHigh,
        }
    }
    pub fn value(self) -> (u8, usize) {
        match self {
            Self::Low => (55, 3_000_000),
            Self::Medium => (70, 6_000_000),
            Self::High => (85, 12_000_000),
            Self::VeryHigh => (95, 24_000_000),
        }
    }
}
/// Saved app choices. Consent, overwrite authorization and destination are transient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub quality: ExportQuality,
    pub include_audio: bool,
    pub output: OutputOptions,
}
impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Mp4,
            quality: ExportQuality::High,
            include_audio: true,
            output: OutputOptions::default(),
        }
    }
}
impl ExportOptions {
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
    pub saved: Option<PathBuf>,
    pub attempt: Option<ExportOptions>,
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
    temp: Option<PathBuf>,
    scene_outputs: std::collections::VecDeque<PathBuf>,
    attempt_project: Option<Arc<project::Project>>,
    attempt_encoder: EncodePreference,
    pub(crate) selection_range: Option<time::TimeRange>,
    save_index: usize,
    finished: Option<PathBuf>,
    pending_save: bool,
    attempt: Option<ExportOptions>,
    diagnostics: Option<String>,
    saved: Option<PathBuf>,
    publishing: Option<PathBuf>,
    input: Option<CacheKey>,
    publication_fs: Arc<dyn project::atomic::AtomicFs + Send + Sync>,
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
            attempt_project: None,
            attempt_encoder: EncodePreference::default(),
            selection_range: None,
            save_index: 0,
            finished: None,
            pending_save: false,
            attempt: None,
            diagnostics: None,
            saved: None,
            publishing: None,
            input: None,
            publication_fs: Arc::new(project::atomic::Fs),
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
            finished: self.finished.clone(),
            diagnostics: self.diagnostics.clone(),
            saved: self.saved.clone(),
            attempt: self.attempt,
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
                self.cancelling = false;
                if let Some(path) = self.publishing.take() {
                    if let Some(finished) = self.finished.take() {
                        remove_output(&finished);
                    }
                    self.saved = Some(path);
                    self.save_index = self.save_index.saturating_add(1);
                    self.finished = self.scene_outputs.pop_front();
                    self.pending_save = self.finished.is_some();
                    self.open = self.finished.is_some();
                    self.snapshot = None;
                    self.job = None;
                    self.diagnostics = None;
                    return Ok(());
                }
                if self.finished.is_none() {
                    self.finished = self.temp.take();
                    self.pending_save = self.finished.is_some();
                }
            }
            Some(JobState::Failed) => {
                if self.publishing.is_none() {
                    self.discard_scene_outputs();
                }
                if self.publishing.is_none()
                    && let Some(path) = self.temp.take()
                {
                    remove_output(&path);
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
                if self.publishing.is_none() {
                    self.discard_scene_outputs();
                }
                if let Some(temp) = self.temp.take() {
                    remove_output(&temp);
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
    pub fn start(
        &mut self,
        project: Arc<project::Project>,
        options: ExportOptions,
        encoder: EncodePreference,
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
        let path = temp_output(options.format);
        let paths = options.output.paths(&project, &path)?;
        let mut settings = options.settings(&project);
        settings.encoder = encoder;
        settings.render = self.render_state.settings(&project)?;
        options.output.apply(&mut settings)?;
        settings.overwrite = project::atomic::Overwrite::Replace;
        let same = self
            .refused
            .as_ref()
            .is_some_and(|(p, a)| Arc::ptr_eq(p, &project) && *a == options.include_audio);
        settings.allow_degraded = self.consent && same && !self.unavailable.is_empty();
        if settings.allow_degraded {
            settings.degraded_consent = Some(self.unavailable.clone());
        }
        let strict = !settings.allow_degraded;
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
        if strict {
            self.consent = false;
            self.unavailable.clear();
        }
        self.snapshot = self.runner.snapshot(job.0);
        self.job = Some(job);
        self.cancelling = false;
        self.error = None;
        self.discard_finished();
        let mut paths = std::collections::VecDeque::from(paths);
        self.temp = paths.pop_front();
        self.scene_outputs = paths;
        self.attempt_project = Some(project.clone());
        self.attempt_encoder = encoder;
        self.save_index = 0;
        self.finished = None;
        self.pending_save = false;
        self.diagnostics = None;
        self.attempt = Some(options);
        self.input = Some(key);
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
        self.start(
            self.attempt_project.clone().unwrap_or(project),
            options,
            self.attempt_encoder,
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
    pub fn save_name(&self, project: &project::Project, options: ExportOptions) -> String {
        let options = self.attempt.unwrap_or(options);
        let project = self.attempt_project.as_deref().unwrap_or(project);
        let mut name = project.data().name.as_str().to_owned();
        if options.output.scene == SceneChoice::All
            && let Some(scene) = project.data().scenes.iter().nth(self.save_index)
        {
            name.push_str(&format!("-scene-{}", scene.id.0));
        }
        options.format.file_name(&name)
    }
    pub fn save_to(&mut self, path: PathBuf, protected: Vec<PathBuf>) -> Result<()> {
        if self.active() {
            return Err(Error::Invalid("export in progress"));
        }
        let source = self
            .finished
            .clone()
            .ok_or(Error::Invalid("no finished export"))?;
        let project = self
            .refused
            .as_ref()
            .ok_or(Error::Invalid("no export snapshot"))?
            .0
            .clone();
        let key = self.input.ok_or(Error::Invalid("no export job"))?;
        let sequence = self
            .attempt
            .is_some_and(|attempt| attempt.format.is_sequence());
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
    pub fn save_cancelled(&mut self) {
        self.pending_save = false;
    }
    pub fn take_saved(&mut self) -> Option<PathBuf> {
        self.saved.take()
    }
    pub fn discard_finished(&mut self) {
        self.discard_scene_outputs();
        if let Some(path) = self.finished.take() {
            remove_output(&path);
        }
        self.pending_save = false;
    }
    fn discard_scene_outputs(&mut self) {
        for path in self.scene_outputs.drain(..) {
            remove_output(&path);
        }
    }
}

fn temp_output(format: ExportFormat) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let name = format!("vibecut-export-{}-{nanos}", std::process::id());
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
}
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
        preview: Option<&egui::TextureHandle>,
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
                    900.0_f32.min(ctx.content_rect().width() - 48.0) - frame_margin.sum().x,
                );
                let dialog_height = 820.0_f32.min(ctx.content_rect().height() - 48.0);
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
                        let rate = options.output.bit_rate.unwrap_or(options.quality.value().1) as f64
                            + if options.include_audio {
                                192_000.0
                            } else {
                                0.0
                            };
                        ui.label(format!(
                            "{} | {} {} MB",
                            i18n::export_duration(locale, duration),
                            tr(Text::ExportEstimatedSize),
                            i18n::decimal(locale, duration * rate / 8_000_000.0, 1)
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
                                    ui.label(tr(if self.cancelling {
                                        Text::ExportCancelling
                                    } else {
                                        Text::ExportProgress
                                    }));
                                    job_progress(ui, "export.progress", snapshot, locale);
                                }
                            }
                            ExportPhase::Failed => {
                                let label = self
                                    .snapshot
                                    .as_ref()
                                    .and_then(|snapshot| snapshot.error.as_deref())
                                    .map_or(Text::ExportFailure, export_failure);
                                ui.colored_label(theme::error_text(dark), tr(label));
                            }
                            ExportPhase::Ready | ExportPhase::Settings => {
                                if self.error.is_some() {
                                    ui.colored_label(theme::error_text(dark), tr(Text::ExportFailure));
                                }
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
                            let wide = ui.available_width() >= 800.0;
                            let leading = if wide { 108.0 - frame_margin.left } else { 0.0 };
                            let gap = if wide { 114.0 } else { 20.0 };
                            let settings_width = ui.available_width() - leading - 227.0 - gap;
                            ui.add_space(leading);
                            ui.allocate_ui_with_layout(
                                egui::vec2(227.0, body_height),
                                egui::Layout::top_down(egui::Align::Center),
                                |ui| {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(
                                            227.0,
                                            (227.0 * project.data().canvas.height as f32
                                                / project.data().canvas.width as f32)
                                                .min(body_height),
                                        ),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(rect, 4.0, theme::window(dark));
                                    if let Some(texture) = preview {
                                        let size = texture.size_vec2();
                                        let scale = (rect.width() / size.x).min(rect.height() / size.y);
                                        let image =
                                            egui::Rect::from_center_size(rect.center(), size * scale);
                                        ui.painter().image(
                                            texture.id(),
                                            image,
                                            egui::Rect::from_min_max(
                                                egui::Pos2::ZERO,
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            egui::Color32::WHITE,
                                        );
                                    } else {
                                        let galley = ui.painter().layout(
                                            tr(if self.empty {
                                                Text::EmptyPreview
                                            } else {
                                                Text::Loading
                                            })
                                            .to_owned(),
                                            egui::FontId::proportional(14.0),
                                            theme::secondary(dark),
                                            rect.width() - 24.0,
                                        );
                                        let position = rect.center() - galley.size() * 0.5;
                                        ui.painter()
                                            .galley(position, galley, theme::secondary(dark));
                                    }
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
            .and_then(|asset| asset.path().file_name())
        {
            ui.add(egui::Label::new(name.to_string_lossy()).truncate());
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
        if let Some(path) = self.temp.take() {
            remove_output(&path);
        }
    }
}
