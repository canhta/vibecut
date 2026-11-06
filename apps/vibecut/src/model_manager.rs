//! Local model manager: catalog presentation, explicit download consent and state.
//!
//! Transfers, checkpoints, cache accounting and integrity stay with engine/ai;
//! this module only presents them and requires a click before granting consent.
use crate::{
    Error, Result,
    catalog::{self, Locale, Text},
    egui, kit as theme,
};
use ai::models::{DownloadConsent, Model};
use engine::{
    ByteBudget, EditError,
    jobs::{
        CancelToken, JobId, JobRunner, JobSnapshot, JobStage, JobState, RunnerConfig, SystemClock,
    },
    models::ModelTransfers,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

/// A user-facing download failure, independent of engine's internal error tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelProblem {
    StorageFull,
    Integrity,
    Network,
    Busy,
    Corrupt,
    Cancelled,
    Failed,
}
/// Snapshot for tests and agents; it never contains provider bodies or paths.
#[derive(Clone)]
pub struct ModelInspection {
    pub open: bool,
    pub selected: Option<String>,
    pub checking: bool,
    pub installed: Vec<String>,
    pub accepted: Vec<String>,
    pub resume: Vec<String>,
    pub active: Option<String>,
    pub job: Option<JobSnapshot>,
    pub problem: Option<ModelProblem>,
    pub problem_model: Option<String>,
}
impl std::fmt::Debug for ModelInspection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModelInspection")
            .field("open", &self.open)
            .field("selected", &self.selected)
            .field("checking", &self.checking)
            .field("installed", &self.installed)
            .field("accepted", &self.accepted)
            .field("resume", &self.resume)
            .field("active", &self.active)
            .field("state", &self.job.as_ref().map(|job| job.state))
            .field("progress", &self.job.as_ref().map(|job| job.progress))
            .field("problem", &self.problem)
            .field("problem_model", &self.problem_model)
            .finish()
    }
}
struct Download {
    model: String,
    id: JobId,
    cancel: CancelToken,
    snapshot: JobSnapshot,
    cancelling: bool,
}
struct CheckOutcome {
    installed: BTreeMap<String, bool>,
    errors: BTreeMap<String, Arc<EditError>>,
}
struct CheckHooks(Arc<AtomicBool>);
impl ai::ExecutionHooks for CheckHooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn stage(&self, _: ai::Stage) {}
}
/// Presentation and composition root for the one model cache shared with the CLI.
pub struct ModelManager {
    open: bool,
    models: Vec<Model>,
    suggested: BTreeSet<String>,
    selected: Option<String>,
    installed: BTreeMap<String, bool>,
    installed_errors: BTreeMap<String, Arc<EditError>>,
    checking: bool,
    check_rx: Option<Receiver<std::result::Result<CheckOutcome, EditError>>>,
    check_cancel: Arc<AtomicBool>,
    check_worker: Option<std::thread::JoinHandle<()>>,
    transfers: Arc<ModelTransfers>,
    runner: JobRunner,
    job: Option<Download>,
    accepted: BTreeSet<String>,
    resumable: BTreeSet<String>,
    problem: Option<ModelProblem>,
    problem_model: Option<String>,
    problem_error: Option<Arc<EditError>>,
}
pub(crate) struct ModelUi {
    pub(crate) widgets: Vec<(String, egui::Response)>,
}
impl Drop for ModelManager {
    fn drop(&mut self) {
        self.check_cancel.store(true, Ordering::Release);
        if let Some(worker) = self.check_worker.take() {
            let _ = worker.join();
        }
    }
}
impl ModelManager {
    /// Open the single shared cache and the pinned ai catalog. No files are fetched.
    pub fn open_default(budget: ByteBudget) -> Result<Self> {
        let (transfers, _) = ModelTransfers::open_default(
            budget.clone(),
            128,
            Arc::new(ai::transport::UreqTransport::new()),
        )?;
        Self::with_transfers(budget, ai::models::catalog()?, transfers)
    }
    /// Injection boundary for tests and composition: the caller owns cache/transport/IO.
    pub fn with_transfers(
        budget: ByteBudget,
        models: Vec<Model>,
        transfers: ModelTransfers,
    ) -> Result<Self> {
        let mut ids = BTreeSet::new();
        for model in &models {
            model.validate()?;
            if !ids.insert(model.id.clone()) {
                return Err(Error::Invalid("duplicate model id"));
            }
        }
        let mut suggested = BTreeSet::new();
        for language in ["en", "vi"] {
            let tag = captions::LanguageTag::parse(language)
                .map_err(|error| Error::Engine(EditError::Captions(error)))?;
            if let Ok(model) = ai::models::suggested_transcriber(&tag) {
                suggested.insert(model.id);
            }
        }
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 1,
                retained: 4,
                network_attempts: 1,
                backoff: Duration::from_millis(250),
                progress_interval: Duration::from_millis(20),
            },
            budget,
            Arc::new(SystemClock::default()),
        )?;
        let check_cancel = Arc::new(AtomicBool::new(false));
        let mut manager = Self {
            open: false,
            models,
            suggested,
            selected: None,
            installed: BTreeMap::new(),
            installed_errors: BTreeMap::new(),
            checking: false,
            check_rx: None,
            check_cancel,
            check_worker: None,
            transfers: Arc::new(transfers),
            runner,
            job: None,
            accepted: BTreeSet::new(),
            resumable: BTreeSet::new(),
            problem: None,
            problem_model: None,
            problem_error: None,
        };
        manager.selected = manager.models.first().map(|model| model.id.clone());
        manager.start_installed_check()?;
        Ok(manager)
    }
    fn start_installed_check(&mut self) -> Result<()> {
        if self.models.is_empty() {
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        let models = self.models.clone();
        let transfers = self.transfers.clone();
        let cancel = self.check_cancel.clone();
        self.checking = true;
        let worker = std::thread::Builder::new()
            .name("vibecut-model-check".into())
            .spawn(move || {
                let hooks = CheckHooks(cancel.clone());
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut installed = BTreeMap::new();
                    let mut errors = BTreeMap::new();
                    for model in models {
                        if cancel.load(Ordering::Acquire) {
                            break;
                        }
                        match transfers.installed(&model, &hooks) {
                            Ok(lease) => {
                                installed.insert(model.id, lease.is_some());
                            }
                            Err(error) => {
                                installed.insert(model.id.clone(), false);
                                errors.insert(model.id, Arc::new(error));
                            }
                        }
                    }
                    CheckOutcome { installed, errors }
                }))
                .map_err(|_| EditError::WorkerPanicked);
                let _ = tx.send(outcome);
            })
            .map_err(Error::Io)?;
        self.check_worker = Some(worker);
        self.check_rx = Some(rx);
        Ok(())
    }
    /// Called by `/09` when a configured model is missing; it never downloads.
    pub fn open_model(&mut self, id: &str) -> Result<()> {
        if !self.models.iter().any(|model| model.id == id) {
            return Err(Error::Invalid("unknown model"));
        }
        self.open = true;
        self.selected = Some(id.to_owned());
        Ok(())
    }
    pub fn show(&mut self) {
        if !self.open {
            self.accepted.clear();
        }
        self.open = true;
        if self.selected.is_none() {
            self.selected = self.models.first().map(|model| model.id.clone());
        }
    }
    pub fn close(&mut self) {
        if self.active() {
            return;
        }
        self.open = false;
        self.accepted.clear();
    }
    pub fn active(&self) -> bool {
        self.job.as_ref().is_some_and(|job| {
            !matches!(
                job.snapshot.state,
                JobState::Completed | JobState::Cancelled | JobState::Failed
            )
        })
    }
    pub fn inspect(&self) -> ModelInspection {
        ModelInspection {
            open: self.open,
            selected: self.selected.clone(),
            checking: self.checking,
            installed: self
                .installed
                .iter()
                .filter(|(_, installed)| **installed)
                .map(|(id, _)| id.clone())
                .collect(),
            accepted: self.accepted.iter().cloned().collect(),
            resume: self
                .models
                .iter()
                .filter(|model| self.resumable(&model.id))
                .map(|model| model.id.clone())
                .collect(),
            active: self
                .job
                .as_ref()
                .filter(|_| self.active())
                .map(|job| job.model.clone()),
            job: self.job.as_ref().map(|job| job.snapshot.clone()),
            problem: self.problem,
            problem_model: self.problem_model.clone(),
        }
    }
    /// Pump the engine runner and installed-model checks; no IO happens without a click.
    pub fn tick(&mut self) -> Result<()> {
        self.poll_check();
        if let Err(error) = self.runner.tick() {
            self.problem = Some(ModelProblem::Failed);
            self.problem_model = self.job.as_ref().map(|job| job.model.clone());
            self.problem_error = Some(Arc::new(error));
            return Ok(());
        }
        let _ = self.runner.drain_events();
        let (model, snapshot) = {
            let Some(job) = &mut self.job else {
                return Ok(());
            };
            if let Some(snapshot) = self.runner.snapshot(job.id) {
                job.snapshot = snapshot;
            }
            if !terminal(job.snapshot.state) {
                return Ok(());
            }
            job.cancelling = false;
            (job.model.clone(), job.snapshot.clone())
        };
        match snapshot.state {
            JobState::Completed => {
                self.installed.insert(model.clone(), true);
                self.installed_errors.remove(&model);
                self.accepted.remove(&model);
                self.resumable.remove(&model);
                self.problem = None;
                self.problem_model = None;
                self.problem_error = None;
            }
            JobState::Cancelled => {
                self.resumable.insert(model.clone());
                self.problem = Some(ModelProblem::Cancelled);
                self.problem_model = Some(model);
                self.problem_error = snapshot.error.clone();
            }
            JobState::Failed => {
                let problem = snapshot
                    .error
                    .as_deref()
                    .map_or(ModelProblem::Failed, problem_for);
                if problem == ModelProblem::Network {
                    self.resumable.insert(model.clone());
                } else {
                    self.resumable.remove(&model);
                }
                self.installed.insert(model.clone(), false);
                self.problem = Some(problem);
                self.problem_model = Some(model);
                self.problem_error = snapshot.error.clone();
            }
            _ => {}
        }
        Ok(())
    }
    fn poll_check(&mut self) {
        let result = match self.check_rx.as_ref().map(Receiver::try_recv) {
            Some(Ok(result)) => Some(result),
            Some(Err(TryRecvError::Empty)) => None,
            Some(Err(TryRecvError::Disconnected)) => {
                self.checking = false;
                self.check_rx = None;
                self.problem = Some(ModelProblem::Failed);
                None
            }
            None => None,
        };
        let Some(result) = result else {
            return;
        };
        self.checking = false;
        self.check_rx = None;
        match result {
            Ok(outcome) => {
                self.installed = outcome.installed;
                self.installed_errors = outcome.errors;
            }
            Err(error) => {
                self.problem = Some(problem_for(&error));
                self.problem_error = Some(Arc::new(error));
            }
        }
    }
    fn start(&mut self, id: &str) -> Result<()> {
        if self.active() {
            return Err(Error::Invalid("a model download is already running"));
        }
        if self.checking {
            return Err(Error::Invalid("model check is still running"));
        }
        let model = self
            .models
            .iter()
            .find(|model| model.id == id)
            .cloned()
            .ok_or(Error::Invalid("unknown model"))?;
        if self.installed.get(id).copied().unwrap_or(false) {
            return Err(Error::Invalid("model is already installed"));
        }
        if !model.default_eligible() && !self.accepted.contains(id) {
            return Err(Error::Invalid("model licence consent is required"));
        }
        // This is the affirmative action: consent is granted only after this click.
        let consent = DownloadConsent::grant(&model)?;
        let (job, cancel) =
            self.transfers
                .submit(&mut self.runner, model.clone(), Some(consent))?;
        let snapshot = self
            .runner
            .snapshot(job)
            .ok_or(Error::Invalid("job identity"))?;
        self.job = Some(Download {
            model: model.id,
            id: job,
            cancel,
            snapshot,
            cancelling: false,
        });
        self.problem = None;
        self.problem_model = None;
        self.problem_error = None;
        Ok(())
    }
    fn cancel(&mut self) {
        if let Some(job) = &mut self.job {
            let active = !terminal(job.snapshot.state);
            job.cancel.cancel();
            job.cancelling = active;
        }
    }
    fn model(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|model| model.id == id)
    }
    fn resumable(&self, id: &str) -> bool {
        self.resumable.contains(id)
            || (self.problem_model.as_deref() == Some(id)
                && matches!(
                    self.problem,
                    Some(ModelProblem::Network | ModelProblem::Cancelled)
                ))
    }
    fn status<'a>(&'a self, id: &'a str) -> Text {
        if self.checking && !self.installed.contains_key(id) {
            return Text::ModelChecking;
        }
        if self.installed_errors.contains_key(id) {
            return Text::ModelFailed;
        }
        if self.installed.get(id).copied().unwrap_or(false) {
            return Text::ModelInstalled;
        }
        if let Some(job) = &self.job
            && job.model == id
            && self.active()
        {
            return if job.cancelling {
                Text::ModelCancelling
            } else {
                match job.snapshot.stage {
                    JobStage::Verify | JobStage::Publish => Text::ModelVerifying,
                    _ => Text::ModelDownloading,
                }
            };
        }
        if self.problem_model.as_deref() == Some(id) {
            return match self.problem {
                Some(ModelProblem::Cancelled) => Text::ModelCancelled,
                Some(_) => Text::ModelFailed,
                None => Text::ModelNotInstalled,
            };
        }
        Text::ModelNotInstalled
    }
    /// Draw the modal and return every widget for Editor's stable automation map.
    pub(crate) fn ui(&mut self, ctx: &egui::Context, locale: Locale, dark: bool) -> ModelUi {
        let mut output = ModelUi {
            widgets: Vec::new(),
        };
        if let Err(error) = self.tick() {
            self.problem = Some(ModelProblem::Failed);
            self.problem_error = Some(Arc::new(error_from(error)));
        }
        if !self.open {
            return output;
        }
        let tr = |key| catalog::text(locale, key);
        let modal = egui::Modal::new(egui::Id::unique("models.dialog"))
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .corner_radius(12)
                    .inner_margin(24),
            )
            .show(ctx, |ui| {
                ui.set_width(880.0_f32.min(ctx.content_rect().width() - 80.0));
                ui.label(egui::RichText::new(tr(Text::Models)).strong().size(18.0));
                ui.weak(tr(Text::ModelsHint));
                ui.add_space(12.0);
                let body_height = (ctx.content_rect().height() - 300.0).clamp(260.0, 520.0);
                let list_height = (body_height * 0.35).clamp(90.0, 150.0);
                egui::ScrollArea::vertical()
                    .id_salt("models.list")
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let ids: Vec<String> =
                            self.models.iter().map(|model| model.id.clone()).collect();
                        for id in ids {
                            let selected = self.selected.as_deref() == Some(id.as_str());
                            let status = self.status(&id);
                            ui.horizontal(|ui| {
                                let response = ui.add_sized(
                                    [ui.available_width() - 100.0, 26.0],
                                    egui::Button::selectable(
                                        selected,
                                        egui::RichText::new(&id).color(if selected {
                                            theme::on_accent(dark)
                                        } else {
                                            theme::text(dark)
                                        }),
                                    ),
                                );
                                if response.clicked() {
                                    self.selected = Some(id.clone());
                                }
                                record(&mut output.widgets, format!("models.row.{id}"), response);
                                ui.label(egui::RichText::new(tr(status)).small().color(
                                    if status == Text::ModelFailed {
                                        theme::error_text(dark)
                                    } else {
                                        theme::secondary(dark)
                                    },
                                ));
                            });
                            ui.add_space(2.0);
                        }
                    });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("models.detail")
                    .max_height((body_height - list_height - 20.0).max(80.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.detail(ui, locale, dark, &mut output.widgets));
                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);
                self.footer(ui, locale, dark, &mut output.widgets);
            });
        if modal.should_close() {
            self.close();
        }
        output
    }
    fn detail(
        &mut self,
        ui: &mut egui::Ui,
        locale: Locale,
        dark: bool,
        widgets: &mut Vec<(String, egui::Response)>,
    ) {
        let tr = |key| catalog::text(locale, key);
        let Some(id) = self.selected.clone() else {
            ui.weak(tr(Text::ModelChoose));
            return;
        };
        let Some(model) = self.model(&id).cloned() else {
            ui.weak(tr(Text::ModelChoose));
            return;
        };
        ui.label(egui::RichText::new(&model.id).strong().size(18.0));
        if model.default_eligible() && self.suggested.contains(&model.id) {
            ui.colored_label(theme::accent(dark), tr(Text::ModelSuggested));
        }
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tr(Text::ModelProvider)).strong());
            ui.label(tr(Text::ModelOnDevice));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tr(Text::ModelLanguages)).strong());
            if model.language_neutral() {
                ui.label(tr(Text::ModelLanguageNeutral));
            } else {
                ui.label(
                    model
                        .languages
                        .iter()
                        .map(captions::LanguageTag::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
        });
        let total = model
            .files
            .iter()
            .try_fold(0u64, |sum, file| sum.checked_add(file.size))
            .unwrap_or(u64::MAX);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tr(Text::ModelSize)).strong());
            ui.label(catalog::format(
                locale,
                Text::ModelSizeValue,
                &[
                    ("size", &human_size(locale, total)),
                    ("bytes", &catalog::number(locale, total)),
                ],
            ));
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tr(Text::ModelLicence)).strong());
            ui.hyperlink_to(&model.license, &model.license_url);
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tr(Text::ModelSource)).strong());
            ui.hyperlink_to(&model.source, &model.source);
        });
        ui.add_space(8.0);
        if model.default_eligible() {
            ui.label(tr(Text::ModelPermissive));
        } else {
            ui.colored_label(theme::warning(dark), tr(Text::ModelRestrictions));
            let mut accepted = self.accepted.contains(&id);
            let response = ui.checkbox(&mut accepted, tr(Text::ModelAccept));
            if response.changed() {
                if accepted {
                    self.accepted.insert(id.clone());
                } else {
                    self.accepted.remove(&id);
                }
            }
            record(widgets, format!("models.accept.{id}"), response);
        }
        ui.add_space(10.0);
        ui.label(egui::RichText::new(tr(Text::ModelFiles)).strong());
        for file in &model.files {
            ui.label(catalog::format(
                locale,
                Text::ModelFileLine,
                &[
                    ("name", &file.name),
                    ("size", &human_size(locale, file.size)),
                ],
            ));
            ui.label(
                egui::RichText::new(&file.sha256)
                    .small()
                    .monospace()
                    .color(theme::secondary(dark)),
            );
        }
        if let Some(error) = self.installed_errors.get(&id).cloned() {
            ui.colored_label(
                theme::error_text(dark),
                tr(problem_text(problem_for(&error))),
            );
        }
        if self.installed.get(&id).copied().unwrap_or(false) {
            ui.colored_label(theme::success(dark), tr(Text::ModelInstalled));
        }
    }
    fn footer(
        &mut self,
        ui: &mut egui::Ui,
        locale: Locale,
        dark: bool,
        widgets: &mut Vec<(String, egui::Response)>,
    ) {
        let tr = |key| catalog::text(locale, key);
        if self.checking {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr(Text::ModelChecking));
            });
        }
        if let Some(problem) = self.problem {
            ui.colored_label(
                if problem == ModelProblem::Cancelled {
                    theme::secondary(dark)
                } else {
                    theme::error_text(dark)
                },
                tr(problem_text(problem)),
            );
            if let Some(error) = &self.problem_error {
                ui.weak(error.to_string());
            }
        }
        if let Some(job) = self.job.as_ref().filter(|_| self.active()) {
            let stage = match job.snapshot.stage {
                JobStage::Verify => Text::ModelVerifying,
                JobStage::Publish => Text::ModelPublishing,
                JobStage::Backoff => Text::ModelQueued,
                _ => Text::ModelDownloading,
            };
            ui.label(catalog::format(
                locale,
                Text::ModelProgress,
                &[("model", &job.model), ("stage", tr(stage))],
            ));
            ui.add(egui::ProgressBar::new(job.snapshot.progress).show_percentage());
        }
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let active = self.active();
                let close = ui.add_enabled(
                    !active,
                    egui::Button::new(tr(Text::ModelClose)).min_size(egui::vec2(90.0, 34.0)),
                );
                if close.clicked() {
                    self.open = false;
                    self.accepted.clear();
                }
                record(widgets, "models.close".into(), close);
                if active {
                    let cancelling = self.job.as_ref().is_some_and(|job| job.cancelling);
                    let cancel = ui.add_enabled(
                        !cancelling,
                        egui::Button::new(tr(Text::ModelCancel)).min_size(egui::vec2(90.0, 34.0)),
                    );
                    if cancel.clicked() {
                        self.cancel();
                    }
                    record(widgets, "models.cancel".into(), cancel);
                } else if let Some(id) = self.selected.clone()
                    && let Some(model) = self.model(&id).cloned()
                    && !self.installed.get(&id).copied().unwrap_or(false)
                {
                    let consent = model.default_eligible() || self.accepted.contains(&id);
                    let enabled = !self.checking && consent;
                    let label = if self.resumable(&id) {
                        Text::ModelResume
                    } else {
                        Text::ModelDownload
                    };
                    let response = ui.add_enabled(
                        enabled,
                        egui::Button::new(
                            egui::RichText::new(tr(label)).color(theme::on_accent(dark)),
                        )
                        .fill(theme::accent(dark))
                        .min_size(egui::vec2(120.0, 34.0)),
                    );
                    if response.clicked()
                        && let Err(error) = self.start(&id)
                    {
                        self.problem = Some(problem_for(&error_from(error)));
                        self.problem_model = Some(id.clone());
                    }
                    record(widgets, format!("models.download.{id}"), response);
                }
            });
        });
    }
}
fn terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Cancelled | JobState::Failed
    )
}
fn record(widgets: &mut Vec<(String, egui::Response)>, id: String, response: egui::Response) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(&id));
    widgets.push((id, response));
}
fn error_from(error: Error) -> EditError {
    match error {
        Error::Engine(error) => error,
        Error::Ai(error) => EditError::Ai(error),
        _ => EditError::Invalid("model download"),
    }
}
fn problem_for(error: &EditError) -> ModelProblem {
    match error {
        EditError::StorageFull
        | EditError::Ai(
            ai::Error::StorageFull | ai::Error::ModelStorage(std::io::ErrorKind::StorageFull),
        ) => ModelProblem::StorageFull,
        EditError::Ai(ai::Error::ModelIntegrity) => ModelProblem::Integrity,
        EditError::Ai(
            ai::Error::Network
            | ai::Error::Timeout
            | ai::Error::Server
            | ai::Error::RateLimit { .. },
        )
        | EditError::NetworkTransient => ModelProblem::Network,
        EditError::CacheBusy => ModelProblem::Busy,
        EditError::CacheCorrupt => ModelProblem::Corrupt,
        EditError::Cancelled => ModelProblem::Cancelled,
        EditError::Io(error) if error.kind() == std::io::ErrorKind::StorageFull => {
            ModelProblem::StorageFull
        }
        _ => ModelProblem::Failed,
    }
}
fn problem_text(problem: ModelProblem) -> Text {
    match problem {
        ModelProblem::StorageFull => Text::ModelStorageFull,
        ModelProblem::Integrity => Text::ModelIntegrity,
        ModelProblem::Network => Text::ModelNetwork,
        ModelProblem::Busy => Text::ModelBusy,
        ModelProblem::Corrupt => Text::ModelCorrupt,
        ModelProblem::Cancelled => Text::ModelCancelled,
        ModelProblem::Failed => Text::ModelFailed,
    }
}
fn human_size(locale: Locale, bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    let unit = UNITS.get(unit).copied().unwrap_or("B");
    format!("{} {unit}", catalog::decimal(locale, value, 2))
}
