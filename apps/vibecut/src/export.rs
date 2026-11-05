//! Export presentation and saved choices over the engine's existing export job.
use crate::{
    Error, Result,
    catalog::{self, Locale, Text},
    egui, theme,
};
use engine::{
    ByteBudget,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{
        CancelToken, JobId, JobRunner, JobSnapshot, JobStage, JobState, RunnerConfig, SystemClock,
    },
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    #[default]
    Mp4,
    Mov,
    Webm,
}
impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Webm => "webm",
        }
    }
    pub fn container(self) -> media::Container {
        match self {
            Self::Mp4 => media::Container::Mp4,
            Self::Mov => media::Container::Mov,
            Self::Webm => media::Container::WebM,
        }
    }
    fn label(self) -> Text {
        match self {
            Self::Mp4 => Text::ExportMp4,
            Self::Mov => Text::ExportMov,
            Self::Webm => Text::ExportWebm,
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
    fn label(self) -> Text {
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
}
impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Mp4,
            quality: ExportQuality::High,
            include_audio: true,
        }
    }
}
impl ExportOptions {
    pub fn settings(self, project: &project::Project) -> render::ExportSettings {
        let mut settings = render::ExportSettings::from_project(project);
        settings.container = self.format.container();
        (settings.quality, settings.bit_rate) = self.quality.value();
        settings.include_audio = self.include_audio;
        // A chosen quality must be applied, rather than bypassed by Auto copy.
        settings.mode = render::ExportMode::Encode;
        settings
    }
}
#[derive(Clone)]
pub struct ExportInspection {
    pub open: bool,
    pub destination: Option<PathBuf>,
    pub job: Option<JobSnapshot>,
    pub unavailable: Vec<render::Unavailable>,
    pub consent: bool,
    pub replace: bool,
    pub empty: bool,
    pub error: Option<Arc<Error>>,
}
pub struct ExportDialog {
    pub(crate) open: bool,
    destination: Option<PathBuf>,
    replace: bool,
    consent: bool,
    unavailable: Vec<render::Unavailable>,
    refused: Option<(Arc<project::Project>, bool)>,
    empty: bool,
    error: Option<Arc<Error>>,
    runner: JobRunner,
    sources: Arc<dyn crate::playback::Sources>,
    job: Option<(JobId, CancelToken)>,
    snapshot: Option<JobSnapshot>,
    cancelling: bool,
}
impl ExportDialog {
    pub fn new(budget: ByteBudget) -> Result<Self> {
        Ok(Self {
            open: false,
            destination: None,
            replace: false,
            consent: false,
            unavailable: Vec::new(),
            refused: None,
            empty: false,
            error: None,
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
            sources: Arc::new(crate::playback::Files),
            job: None,
            snapshot: None,
            cancelling: false,
        })
    }
    /// Inject only the render source boundary; scheduling stays with engine.
    pub fn set_sources(&mut self, sources: Arc<dyn crate::playback::Sources>) -> Result<()> {
        if self.active() {
            return Err(Error::Invalid("export in progress"));
        }
        self.sources = sources;
        Ok(())
    }
    pub fn inspect(&self) -> ExportInspection {
        ExportInspection {
            open: self.open,
            destination: self.destination.clone(),
            job: self.snapshot.clone(),
            unavailable: self.unavailable.clone(),
            consent: self.consent,
            replace: self.replace,
            empty: self.empty,
            error: self.error.clone(),
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
            self.replace = false;
        }
        self.open = true;
        self.empty = project.duration() == time::Tick::ZERO;
    }
    /// Inject the native save picker's result; choosing a path never authorizes replacement.
    pub fn set_destination(&mut self, path: PathBuf) -> Result<()> {
        if self.active() {
            return Err(Error::Invalid("export in progress"));
        }
        if path.as_os_str().is_empty() || path.as_os_str().len() > 32768 {
            return Err(Error::Invalid("export destination"));
        }
        self.destination = Some(path);
        self.replace = false;
        self.error = None;
        Ok(())
    }
    pub fn cancel(&mut self) {
        if let Some((_, token)) = &self.job {
            token.cancel();
            self.cancelling = self.active();
        }
    }
    pub fn tick(&mut self, project: &Arc<project::Project>, options: ExportOptions) -> Result<()> {
        self.runner.tick()?;
        self.runner.drain_events();
        if let Some((id, _)) = self.job {
            self.snapshot = self.runner.snapshot(id);
        }
        if self
            .refused
            .as_ref()
            .is_some_and(|(p, a)| !Arc::ptr_eq(p, project) || *a != options.include_audio)
        {
            self.consent = false;
            self.unavailable.clear();
            if !self.active() {
                self.snapshot = None;
                self.job = None;
                self.refused = None;
            }
            return Ok(());
        }
        if let Some(snapshot) = &self.snapshot {
            if !self.active() {
                self.cancelling = false;
            }
            if let Some(error) = &snapshot.error
                && let engine::EditError::Render(render::Error::Unavailable(items)) = error.as_ref()
                && self.unavailable != *items
            {
                self.consent = false;
                self.unavailable = items.clone();
            }
        }
        Ok(())
    }
    pub fn start(&mut self, project: Arc<project::Project>, options: ExportOptions) -> Result<()> {
        if self.active() {
            return Err(Error::Invalid("export in progress"));
        }
        self.empty = project.duration() == time::Tick::ZERO;
        if self.empty {
            return Err(Error::EmptyTimeline);
        }
        let path = self
            .destination
            .clone()
            .ok_or(Error::Invalid("export destination required"))?;
        let range = time::TimeRange::new(time::Tick::ZERO, project.duration())?;
        let mut settings = options.settings(&project);
        settings.overwrite = if self.replace {
            project::atomic::Overwrite::Replace
        } else {
            project::atomic::Overwrite::Refuse
        };
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
        let job = engine::submit_export_with_sources(
            &mut self.runner,
            key,
            project.clone(),
            range,
            settings,
            path,
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
        self.refused = Some((project, options.include_audio));
        Ok(())
    }
}

pub(crate) enum Action {
    Pick,
    Start,
}
pub(crate) struct DialogUi {
    pub widgets: Vec<(&'static str, egui::Response)>,
    pub action: Option<Action>,
}
impl ExportDialog {
    pub(crate) fn ui(
        &mut self,
        ctx: &egui::Context,
        locale: Locale,
        dark: bool,
        options: &mut ExportOptions,
        project: &project::Project,
        preview: Option<&egui::TextureHandle>,
    ) -> DialogUi {
        let mut result = DialogUi {
            widgets: Vec::new(),
            action: None,
        };
        if !self.open {
            return result;
        }
        let tr = |k| catalog::text(locale, k);
        let active = self.active();
        let modal = egui::Modal::new(egui::Id::unique("export.dialog"))
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .corner_radius(12)
                    .inner_margin(24),
            )
            .show(ctx, |ui| {
                ui.set_width(852.0_f32.min(ctx.content_rect().width() - 96.0));
                ui.label(
                    egui::RichText::new(tr(Text::ExportTitle))
                        .strong()
                        .size(18.0),
                );
                ui.add_space(12.0);
                ui.label(project.data().name.as_str());
                ui.weak(tr(Text::ExportSettingsHint));
                ui.add_space(16.0);
                let body_height = (ctx.content_rect().height() - 360.0).clamp(280.0, 460.0);
                ui.horizontal_top(|ui| {
                    let settings_width =
                        ui.available_width() - 247.0 - 2.0 * ui.spacing().item_spacing.x;
                    ui.allocate_ui_with_layout(
                        egui::vec2(227.0, body_height),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.label(tr(Text::Preview));
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(227.0, body_height - 28.0),
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
                    ui.add_space(20.0);
                    ui.vertical(|ui| {
                        ui.set_width(settings_width);
                        ui.set_min_height(body_height);
                        egui::ScrollArea::vertical()
                            .id_salt("export.settings")
                            .max_height(body_height)
                            .show(ui, |ui| {
                                ui.add_enabled_ui(!active, |ui| {
                                    ui.label(tr(Text::ExportDestination));
                                    if let Some(path) = &self.destination {
                                        ui.add(egui::Label::new(path.to_string_lossy()).truncate())
                                            .on_hover_text(path.to_string_lossy());
                                    }
                                    let r = ui.add(egui::Button::image_and_text(
                                        crate::icons::tinted(
                                            crate::icons::Icon::FolderOpen,
                                            16.0,
                                            if active {
                                                theme::disabled(dark)
                                            } else {
                                                theme::text(dark)
                                            },
                                        ),
                                        tr(Text::ChooseDestination),
                                    ));
                                    if r.clicked() {
                                        result.action = Some(Action::Pick);
                                    }
                                    result.widgets.push(("export.destination", r));
                                    if self.destination.as_ref().is_some_and(|p| p.exists()) {
                                        let r =
                                            ui.checkbox(&mut self.replace, tr(Text::ExportReplace));
                                        result.widgets.push(("export.replace", r));
                                    }
                                    ui.add_space(12.0);
                                    let header = egui::CollapsingHeader::new(format!(
                                        "{} · {}",
                                        tr(Text::ExportFormat),
                                        tr(options.format.label())
                                    ))
                                    .id_salt("export.format")
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        for (format, id) in [
                                            (ExportFormat::Mp4, "export.format.mp4"),
                                            (ExportFormat::Mov, "export.format.mov"),
                                            (ExportFormat::Webm, "export.format.webm"),
                                        ] {
                                            let r = ui.radio_value(
                                                &mut options.format,
                                                format,
                                                tr(format.label()),
                                            );
                                            result.widgets.push((id, r));
                                        }
                                    });
                                    result
                                        .widgets
                                        .push(("export.format", header.header_response));
                                    ui.add_space(14.0);
                                    let header = egui::CollapsingHeader::new(format!(
                                        "{} · {}",
                                        tr(Text::ExportQuality),
                                        tr(options.quality.label())
                                    ))
                                    .id_salt("export.quality")
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        for (quality, id) in [
                                            (ExportQuality::Low, "export.quality.low"),
                                            (ExportQuality::Medium, "export.quality.medium"),
                                            (ExportQuality::High, "export.quality.high"),
                                            (ExportQuality::VeryHigh, "export.quality.very_high"),
                                        ] {
                                            let r = ui.radio_value(
                                                &mut options.quality,
                                                quality,
                                                tr(quality.label()),
                                            );
                                            result.widgets.push((id, r));
                                        }
                                    });
                                    result
                                        .widgets
                                        .push(("export.quality", header.header_response));
                                    ui.add_space(14.0);
                                    let header = egui::CollapsingHeader::new(tr(Text::Audio))
                                        .id_salt("export.audio")
                                        .default_open(false)
                                        .show(ui, |ui| {
                                            let r = ui.checkbox(
                                                &mut options.include_audio,
                                                tr(Text::IncludeAudio),
                                            );
                                            result.widgets.push(("export.audio.include", r));
                                        });
                                    result
                                        .widgets
                                        .push(("export.audio", header.header_response));
                                    ui.add_space(14.0);
                                    if !self.unavailable.is_empty() {
                                        ui.colored_label(
                                            theme::warning(dark),
                                            tr(Text::ExportBlocked),
                                        );
                                        for item in &self.unavailable {
                                            if let Some(name) = project
                                                .data()
                                                .assets
                                                .iter()
                                                .find(|asset| asset.id() == item.asset)
                                                .and_then(|asset| asset.path().file_name())
                                            {
                                                ui.add(
                                                    egui::Label::new(name.to_string_lossy())
                                                        .truncate(),
                                                );
                                            }
                                            let reason = match item.reason {
                                                render::UnavailableReason::Missing => {
                                                    Text::SourceMissing
                                                }
                                                render::UnavailableReason::Changed => {
                                                    Text::SourceChanged
                                                }
                                                render::UnavailableReason::Metadata => {
                                                    Text::SourceMetadata
                                                }
                                                render::UnavailableReason::Video => {
                                                    Text::SourceVideo
                                                }
                                                render::UnavailableReason::Audio => {
                                                    Text::SourceAudio
                                                }
                                            };
                                            ui.label(format!(
                                                "{} {} · {} {} · {}",
                                                tr(Text::SourceAsset),
                                                item.asset.0,
                                                tr(Text::SourceItem),
                                                item.item.0,
                                                tr(reason)
                                            ));
                                            ui.label(catalog::export_range(
                                                locale,
                                                item.range.start().seconds(),
                                                item.range.end().seconds(),
                                            ));
                                        }
                                        let r = ui
                                            .checkbox(&mut self.consent, tr(Text::ExportDegraded));
                                        result.widgets.push(("export.consent", r));
                                    }
                                });
                            });
                    });
                });
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);
                self.empty = project.duration() == time::Tick::ZERO;
                if self.empty {
                    ui.colored_label(theme::error_text(dark), tr(Text::ExportEmpty));
                }
                if let Some(snapshot) = &self.snapshot {
                    let label = match snapshot.state {
                        JobState::Completed => Text::ExportComplete,
                        JobState::Failed => snapshot
                            .error
                            .as_deref()
                            .map_or(Text::ExportFailure, export_failure),
                        JobState::Cancelled => Text::ExportCancelled,
                        _ => {
                            if self.cancelling {
                                Text::ExportCancelling
                            } else {
                                Text::ExportProgress
                            }
                        }
                    };
                    ui.label(tr(label));
                    if active {
                        let stage = match snapshot.stage {
                            JobStage::Queued | JobStage::Starting => Text::ExportQueued,
                            JobStage::Admit => Text::ExportAdmit,
                            JobStage::Render => Text::ExportRender,
                            JobStage::Mix => Text::ExportMix,
                            JobStage::Encode => Text::ExportEncode,
                            JobStage::Finalize => Text::ExportFinalize,
                            JobStage::Publish => Text::ExportPublish,
                            _ => Text::ExportProgress,
                        };
                        ui.label(tr(stage));
                        ui.add(egui::ProgressBar::new(snapshot.progress).show_percentage());
                    }
                }
                if self.error.is_some() {
                    ui.colored_label(theme::error_text(dark), tr(Text::ExportFailure));
                }
                if self.destination.is_none() {
                    ui.weak(tr(Text::ExportChoosePath));
                }
                ui.horizontal(|ui| {
                    ui.label(catalog::export_duration(
                        locale,
                        project.duration().seconds(),
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if active {
                            let r = ui.add_enabled(
                                !self.cancelling,
                                egui::Button::new(tr(Text::Cancel))
                                    .min_size(egui::vec2(90.0, 34.0)),
                            );
                            if r.clicked() {
                                self.cancel();
                            }
                            result.widgets.push(("export.cancel", r));
                        } else {
                            // The render job rechecks sources, including after restoration.
                            let ready = !self.empty
                                && self.destination.is_some()
                                && (!self.destination.as_ref().is_some_and(|p| p.exists())
                                    || self.replace);
                            let r = ui.add_enabled(
                                ready,
                                egui::Button::new(
                                    egui::RichText::new(tr(Text::Export))
                                        .color(theme::on_accent(dark)),
                                )
                                .fill(theme::accent(dark))
                                .min_size(egui::vec2(90.0, 34.0)),
                            );
                            if r.clicked() {
                                result.action = Some(Action::Start);
                            }
                            result.widgets.push(("export.start", r));
                            let r = ui.add_sized(
                                [90.0, 34.0],
                                egui::Button::new(tr(
                                    if self
                                        .snapshot
                                        .as_ref()
                                        .is_some_and(|s| s.state == JobState::Completed)
                                    {
                                        Text::ExportDone
                                    } else {
                                        Text::Cancel
                                    },
                                )),
                            );
                            if r.clicked() {
                                self.open = false;
                            }
                            result.widgets.push(("export.close", r));
                        }
                    });
                });
            });
        if modal.should_close() {
            if active {
                self.cancel();
            } else {
                self.open = false;
            }
        }
        result
    }
    pub(crate) fn failed(&mut self, error: Error) {
        self.error = Some(Arc::new(error));
    }
}

impl std::fmt::Debug for ExportInspection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportInspection")
            .field("open", &self.open)
            .field("state", &self.job.as_ref().map(|s| s.state))
            .field("unavailable", &self.unavailable)
            .field("consent", &self.consent)
            .finish_non_exhaustive()
    }
}

fn export_failure(error: &engine::EditError) -> Text {
    use engine::EditError as E;
    use render::Error as R;
    match error {
        E::BudgetFull | E::ArtifactTooLarge | E::Render(R::Budget { .. } | R::Allocation(_)) => {
            Text::ExportMemory
        }
        E::Render(R::AudioOverload { .. }) => Text::ExportOverload,
        E::Render(R::Unavailable(_)) => Text::ExportBlocked,
        E::Io(_) | E::Render(R::Io(_) | R::Project(project::Error::Io(_))) => Text::ExportStorage,
        E::Render(R::Media(_) | R::Unsupported(_)) => Text::ExportCapability,
        E::Render(R::Text(_) | R::Raster(_)) => Text::ExportTextFailure,
        _ => Text::ExportFailure,
    }
}
