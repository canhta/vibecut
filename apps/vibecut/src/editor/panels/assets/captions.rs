//! Captions panel: presentation and bounded staging through existing engine owners.
mod editing;
mod fillers;
mod find;
mod import;
mod manual;
mod tab;
#[cfg(test)]
mod tests;
mod transcript;
mod view;
pub use crate::services::ai_task::TaskEvents;
use crate::{
    Error, Result, egui,
    i18n::Text,
    preferences::{SOURCE_LANGUAGES, SubtitleOptions as Options},
    services::ai_access::{self, Choice, Opener, TextOpener},
};
use captions::{LanguageTag, NfcText};
use engine::{
    ByteBudget, Command, Session,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{
        CancelToken, JobId, JobKind, JobRunner, JobSpec, JobStage, JobState, RunnerConfig,
        SystemClock,
    },
    providers::UploadRequest,
    transcript::{Cancellation, Stage},
};
use project::{ItemId, ItemParams};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};
use time::Tick;

/// What a running job does when it finishes. Generation and translation are registry
/// jobs whose result the window applies with a further command; import is a local parse.
enum Task {
    Generate {
        start: Tick,
        /// The language was auto-detected, so its tag is checked against the supported set.
        auto: bool,
        /// The caption item the run replaces ("Delete current captions"): `captions.apply`
        /// removes it in the same batch, so the replacement is one undo step.
        replace: Option<ItemId>,
    },
    Translate {
        /// The revision the selection was read at; the result applies only against it.
        revision: engine::control::Revision,
    },
    Import {
        token: CancelToken,
        receive: mpsc::Receiver<std::result::Result<Command, Arc<engine::control::Error>>>,
    },
}
struct Active {
    id: JobId,
    task: Task,
    cancel: Cancellation,
    events: Arc<TaskEvents>,
}
#[derive(Clone, Debug)]
pub struct Inspection {
    pub busy: bool,
    pub cancelling: bool,
    pub job_state: Option<JobState>,
    pub stage: Option<Stage>,
    pub progress: Option<f32>,
    pub selected: Vec<String>,
    pub item: Option<ItemId>,
    pub cue_count: usize,
    pub problem: Option<Text>,
    pub error: Option<Arc<dyn engine::Failure + 'static>>,
    pub completed: Option<usize>,
    pub consent: Option<UploadRequest>,
    /// Words selected in the transcript pane.
    pub transcript_selected: usize,
    /// The refusal reason of the last transcript command, if it was refused.
    pub transcript_refusal: Option<Text>,
}
/// Panel state survives tab and locale changes; only Options enter preferences.
struct BankEdit {
    confirm_pending: bool,
    snapshot: Arc<project::Project>,
    item: Option<ItemId>,
    at: Tick,
    preset: Option<project::CaptionPresetId>,
    name: String,
    save: bool,
    focused: bool,
}

/// Snapshot-bound full-style draft over one live saved preset selection.
struct StyleEdit {
    snapshot: Arc<project::Project>,
    item: ItemId,
    at: Tick,
    preset: project::CaptionPresetId,
    base: captions::CaptionStyle,
    draft: captions::CaptionStyle,
}

struct WordWarnings {
    project: Arc<project::Project>,
    item: ItemId,
    languages: Vec<LanguageTag>,
    fonts: usize,
    used: u64,
    preview: std::result::Result<project::CaptionWordWarnings, Arc<render::Error>>,
    export: std::result::Result<project::CaptionWordWarnings, Arc<render::Error>>,
}

/// Panel state survives tab and locale changes; only Options enter preferences.
pub struct Subtitles {
    pub(crate) models: crate::services::remote_models::AiPreferences,
    bank: Option<BankEdit>,
    style_edit: Option<StyleEdit>,
    bank_frame: Option<u64>,
    word_warnings: Option<WordWarnings>,
    access: Opener,
    text: TextOpener,
    budget: ByteBudget,
    active: Option<Active>,
    classify: Option<fillers::Classify>,
    active_options: Option<Options>,
    export: Option<(JobId, CancelToken)>,
    export_count: Option<usize>,
    runner: JobRunner,
    pub(crate) item: Option<ItemId>,
    selected: BTreeSet<NfcText>,
    anchor: Option<NfcText>,
    page: usize,
    text_focus: Option<egui::Id>,
    /// The Add captions text field, tracked apart from `text_focus` because the cue list resets
    /// that one each frame after the page is drawn.
    manual_focus: Option<egui::Id>,
    drafts: BTreeMap<(NfcText, LanguageTag), String>,
    draft_project: Option<Arc<project::Project>>,
    pub glossary: Vec<ai::GlossaryTerm>,
    force: bool,
    /// Auto captions page: the filler switch shows the filler cut list below the cues.
    identify_fillers: bool,
    /// Auto captions page: the footer checkbox that replaces the current captions on generate.
    delete_current: bool,
    /// Add captions page: the text the next manual cue carries.
    manual_text: String,
    problem: Option<Text>,
    error: Option<Arc<dyn engine::Failure + 'static>>,
    missing_model: Option<String>,
    completed: Option<usize>,
    pub(crate) notice: Option<usize>,
    pub(crate) requested_import: Option<Tick>,
    pub(crate) requested_model: Option<String>,
    pub(crate) requested_seek: Option<Tick>,
    project: Option<project::ProjectId>,
    retry_source: Option<ItemId>,
    fillers: fillers::Cuts,
    find: find::Find,
    transcript: transcript::Pane,
}
impl Drop for Subtitles {
    fn drop(&mut self) {
        self.cancel();
        self.runner.shutdown();
    }
}
impl Subtitles {
    pub fn new(budget: ByteBudget) -> Result<Self> {
        Self::with_access(budget.clone(), ai_access::desktop_opener(budget))
    }
    /// Tests inject the opener at the lent-access seam; the window still sends registry commands.
    pub fn with_access(budget: ByteBudget, access: Opener) -> Result<Self> {
        let runner = job_runner(&budget)?;
        Ok(Self {
            models: Default::default(),
            bank: None,
            style_edit: None,
            bank_frame: None,
            word_warnings: None,
            access,
            text: ai_access::desktop_text_opener(),
            budget,
            classify: None,
            runner,
            active: None,
            active_options: None,
            export: None,
            export_count: None,
            item: None,
            selected: BTreeSet::new(),
            anchor: None,
            page: 0,
            text_focus: None,
            manual_focus: None,
            drafts: BTreeMap::new(),
            draft_project: None,
            glossary: Vec::new(),
            force: false,
            identify_fillers: true,
            delete_current: false,
            manual_text: String::new(),
            problem: None,
            error: None,
            missing_model: None,
            completed: None,
            notice: None,
            requested_import: None,
            requested_model: None,
            requested_seek: None,
            project: None,
            retry_source: None,
            fillers: fillers::Cuts::default(),
            find: Default::default(),
            transcript: Default::default(),
        })
    }
    /// Tests inject the text-provider opener for the filler cut list.
    pub fn with_text_access(mut self, text: TextOpener) -> Self {
        self.text = text;
        self
    }
    pub fn busy(&self) -> bool {
        self.active.is_some() || self.export.is_some() || self.classify.is_some()
    }
    /// The run whose consent sheet is open: the active job, or the filler classification.
    fn consent_events(&self) -> Option<&Arc<TaskEvents>> {
        self.active
            .as_ref()
            .map(|active| &active.events)
            .or_else(|| self.classify.as_ref().map(|run| &run.events))
    }
    pub fn inspect(&self, session: &Session) -> Inspection {
        let state = self
            .consent_events()
            .and_then(|events| events.state.lock().ok());
        let job = self
            .active
            .as_ref()
            .and_then(|a| self.runner.snapshot(a.id));
        Inspection {
            busy: self.busy(),
            job_state: job
                .clone()
                .or_else(|| {
                    self.export
                        .as_ref()
                        .and_then(|(id, _)| self.runner.snapshot(*id))
                })
                .map(|s| s.state),
            cancelling: self.active.as_ref().is_some_and(|a| a.cancel.cancelled()),
            stage: job.as_ref().and_then(|s| stage_of(s.stage)),
            progress: job
                .as_ref()
                .map(|s| s.progress)
                .filter(|p| *p > 0.0)
                .or_else(|| {
                    self.export
                        .as_ref()
                        .and_then(|(id, _)| self.runner.snapshot(*id).map(|s| s.progress))
                }),
            selected: self
                .selected
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect(),
            item: self.item,
            cue_count: self.caption(session).map_or(0, |(_, p)| p.cues.cues.len()),
            problem: self.problem,
            error: self.error.clone(),
            completed: self.completed,
            consent: state
                .as_ref()
                .and_then(|s| s.upload.as_ref().map(|u| u.request.clone())),
            transcript_selected: self.transcript.selected(),
            transcript_refusal: self.transcript.refusal(),
        }
    }
    pub(crate) fn editing(&self, ctx: &egui::Context) -> bool {
        self.style_edit.is_some()
            || (self.bank.is_some()
                && self
                    .bank_frame
                    .is_some_and(|frame| frame + 1 >= ctx.cumulative_frame_nr()))
            || self
                .text_focus
                .is_some_and(|id| ctx.memory(|m| m.focused()) == Some(id))
            || self
                .manual_focus
                .is_some_and(|id| ctx.memory(|m| m.focused()) == Some(id))
    }
    pub(crate) fn modal_open(&self) -> bool {
        self.consent_events()
            .is_some_and(|events| events.state.lock().is_ok_and(|s| s.upload.is_some()))
    }
    pub(crate) fn invalidate_options(&mut self, options: &Options) {
        if self.active.is_some() && self.active_options.as_ref().is_some_and(|o| o != options) {
            self.cancel();
        }
    }
    pub fn cancel(&mut self) {
        if let Some(active) = &self.active {
            active.cancel.cancel();
            match &active.task {
                Task::Import { token, .. } => token.cancel(),
                Task::Generate { .. } | Task::Translate { .. } => {
                    let _ = self.runner.cancel_job(active.id);
                }
            }
        }
        if let Some((_, token)) = &self.export {
            token.cancel();
        }
        if let Some(run) = &self.classify {
            run.cancel.cancel();
        }
    }
    pub fn approve_upload(&mut self, approved: bool) -> Result<()> {
        self.consent_events()
            .ok_or(Error::Invalid("no subtitle job"))?
            .answer(approved)
    }
    /// One job at a time, never inside a gesture; the job's cancellation and consent slot.
    fn begin(&mut self, session: &Session) -> Result<(Cancellation, Arc<TaskEvents>)> {
        if self.busy() || session.transaction_active() {
            return Err(Error::Invalid("subtitle job busy"));
        }
        let cancel = Cancellation::new();
        let events = Arc::new(TaskEvents::new(cancel.clone()));
        Ok((cancel, events))
    }
    fn arm(&mut self, active: Active) {
        self.active = Some(active);
        self.problem = None;
        self.error = None;
        self.completed = None;
        self.missing_model = None;
    }
    pub fn generate(
        &mut self,
        session: &mut Session,
        source: ItemId,
        options: &Options,
    ) -> Result<()> {
        options.validate()?;
        if !options.transcription_supported() {
            return Err(Error::Invalid("subtitle transcription capability"));
        }
        self.retry_source = Some(source);
        let item = session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == source)
            .ok_or(Error::Invalid("select source media"))?;
        // Cues are source-time at 1x; a retimed item needs a rate-aware captions mapping first.
        if !item.retime.speed.is_unity() || item.retime.curve.is_some() {
            return Err(Error::Invalid("captions for a retimed item"));
        }
        let (range, start) = (item.source_range, item.start);
        let asset = session
            .project()
            .data()
            .assets
            .iter()
            .find(|a| Some(a.id()) == item.source_asset)
            .ok_or(Error::Invalid("select source media"))?;
        if !asset
            .probe()
            .streams
            .iter()
            .any(|s| s.kind == media::StreamKind::Audio)
        {
            return Err(Error::Invalid("source has no audio"));
        }
        let asset = asset.id();
        let (cancel, events) = self.begin(session)?;
        let access = (self.access)(
            Choice::Transcribe {
                provider: &options.transcription,
                model: &options.transcription_model,
            },
            events.clone(),
        )?;
        // An omitted language asks the selected provider to detect it as the job's first stage.
        let (id, _) = ai_access::accepted(ai_access::run(
            session,
            &mut self.runner,
            Some(&access),
            None,
            "captions.generate",
            serde_json::json!({
                "asset": asset,
                "range": range,
                "source_language": options.source_tag(),
            }),
        )?)?;
        self.active_options = Some(options.clone());
        self.arm(Active {
            id,
            task: Task::Generate {
                start,
                auto: options.source == "auto",
                replace: self.delete_current.then_some(self.item).flatten(),
            },
            cancel,
            events,
        });
        Ok(())
    }
    /// Re-run the last source with auto-detect after a no-speech/low-confidence
    /// result. The caller's Options keep the new choice for this project.
    pub fn retry_auto(&mut self, session: &mut Session, options: &mut Options) -> Result<()> {
        let source = self.retry_source.ok_or(Error::Invalid("no source media"))?;
        options.source = "auto".into();
        if let Some(project) = self.project {
            options.remember_source(project);
        }
        self.generate(session, source, options)
    }
    /// SRT/VTT parsing is a local command constructor, not a registry job; it runs off the
    /// UI thread under the same admission and result lease as any job.
    pub fn import_file(
        &mut self,
        session: &Session,
        path: PathBuf,
        source: &str,
        at: Tick,
    ) -> Result<()> {
        let asset_id = media::AssetId(
            session
                .project()
                .data()
                .assets
                .iter()
                .map(|a| a.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(Error::Invalid("asset identity overflow"))?,
        );
        let language = LanguageTag::parse(source)?;
        let (cancel, events) = self.begin(session)?;
        let (send, receive) = mpsc::sync_channel(1);
        let worker_cancel = cancel.clone();
        let (id, token) = self.runner.submit_for_session(
            session,
            JobSpec {
                kind: JobKind::Transcribe,
                class: engine::jobs::ResourceClass::Cpu,
                input: key()?,
                bytes: engine::jobs::JobBytes {
                    result: 32 * 1024 * 1024,
                    ..Default::default()
                },
            },
            move |ctx| {
                ctx.check_cancelled()?;
                match import::command(asset_id, path.clone(), language.clone(), at, &worker_cancel)
                {
                    Ok(command) => {
                        send.send(Ok(command)).map_err(|_| {
                            engine::control::Error::Runtime(engine::RuntimeError::Cancelled)
                        })?;
                        Ok(Vec::new())
                    }
                    Err(error) => {
                        let error = Arc::new(error);
                        send.send(Err(error.clone())).map_err(|_| {
                            engine::control::Error::Runtime(engine::RuntimeError::Cancelled)
                        })?;
                        Err(engine::control::Error::Runtime(
                            engine::RuntimeError::Failed(error),
                        ))
                    }
                }
            },
        )?;
        self.active_options = None;
        self.arm(Active {
            id,
            task: Task::Import { token, receive },
            cancel,
            events,
        });
        Ok(())
    }
    pub fn translate(&mut self, session: &mut Session, options: &Options) -> Result<()> {
        engine::providers::choose_translation(
            &options.text_provider,
            Some(&options.translation_model),
        )?;
        let item = self.item.ok_or(Error::Invalid("select captions"))?;
        if self.selected.is_empty() {
            return Err(Error::Invalid("select cues"));
        }
        let (cancel, events) = self.begin(session)?;
        let access = (self.access)(
            Choice::Translate {
                provider: &options.text_provider,
                model: &options.translation_model,
            },
            events.clone(),
        )?;
        let (id, revision) = ai_access::accepted(ai_access::run(
            session,
            &mut self.runner,
            Some(&access),
            None,
            "captions.translate",
            serde_json::json!({
                "item": item,
                "cues": self.selected,
                "target_language": options.target,
                "force": self.force,
                "glossary": self.glossary,
            }),
        )?)?;
        self.active_options = Some(options.clone());
        self.arm(Active {
            id,
            task: Task::Translate { revision },
            cancel,
            events,
        });
        Ok(())
    }
    /// Poll even while another tab is visible. Terminal results never apply after cancellation.
    pub fn tick(&mut self, session: &mut Session) -> Result<bool> {
        self.runner.tick()?;
        self.poll_classify();
        let mut changed = false;
        if self.active.as_ref().is_some_and(|a| {
            self.runner
                .snapshot(a.id)
                .is_some_and(|s| terminal(s.state))
        }) {
            let active = self.active.take().ok_or(Error::Invalid("subtitle job"))?;
            let snapshot = self.runner.snapshot(active.id);
            let state = snapshot.as_ref().map_or(JobState::Failed, |s| s.state);
            let error = snapshot.and_then(|s| s.error);
            if active.cancel.cancelled() || state == JobState::Cancelled {
                self.runner.take_output(active.id);
                self.problem = Some(Text::ShortsJobCancelled);
            } else {
                let created = match active.task {
                    Task::Import { receive, .. } => {
                        let output = self.runner.take_output(active.id);
                        let result = receive.try_recv().unwrap_or_else(|_| {
                            Err(Arc::new(engine::control::Error::Edit(
                                engine::EditError::Invalid("subtitle worker terminated"),
                            )))
                        });
                        match result {
                            Ok(command) => {
                                let output =
                                    output.ok_or(Error::Invalid("subtitle result missing"))?;
                                match output.apply(session, command) {
                                    Ok(outcome) => Some(outcome.created),
                                    Err(e) => {
                                        self.fail(Arc::new(e));
                                        None
                                    }
                                }
                            }
                            Err(e) => {
                                self.fail(e);
                                None
                            }
                        }
                    }
                    task => self.apply_registry(session, active.id, task, state, error),
                };
                if let Some(created) = created {
                    self.item = created
                        .iter()
                        .find_map(|id| {
                            if let engine::CreatedId::Item(id) = id {
                                Some(*id)
                            } else {
                                None
                            }
                        })
                        .or(self.item);
                    self.completed =
                        Some(self.caption(session).map_or(0, |(_, p)| p.cues.cues.len()));
                    self.notice = self.completed;
                    changed = true;
                }
            }
        }
        if let Some((id, _)) = &self.export
            && let Some(snapshot) = self.runner.snapshot(*id)
            && terminal(snapshot.state)
        {
            let id = *id;
            self.export = None;
            match snapshot.state {
                JobState::Completed => self.completed = self.export_count.take(),
                JobState::Cancelled => self.problem = Some(Text::ShortsJobCancelled),
                _ => {
                    self.problem = Some(Text::SubFailed);
                    self.error = snapshot.error.clone();
                }
            }
            if snapshot.state == JobState::Completed {
                self.notice = self.completed;
            }
            self.runner.take_output(id);
        }
        self.refresh(session);
        Ok(changed)
    }
    /// Apply a finished registry job with its ordinary edit command (one undo step): the
    /// retained result is decoded, checked, and sent back through the same admission.
    /// `None` after reporting the refusal; the created ids otherwise.
    fn apply_registry(
        &mut self,
        session: &mut Session,
        id: JobId,
        task: Task,
        state: JobState,
        error: Option<Arc<dyn engine::Failure + 'static>>,
    ) -> Option<Vec<engine::CreatedId>> {
        if state != JobState::Completed {
            self.runner.take_output(id);
            match error {
                Some(error) => self.fail(error),
                None => self.problem = Some(Text::SubFailed),
            }
            return None;
        }
        let applied = self.apply_output(session, id, task);
        self.runner.take_output(id);
        match applied {
            Ok(created) => Some(created),
            Err(error) => {
                self.fail(Arc::new(error));
                None
            }
        }
    }
    fn apply_output(
        &mut self,
        session: &mut Session,
        id: JobId,
        task: Task,
    ) -> engine::control::Result<Vec<engine::CreatedId>> {
        let bytes = self
            .runner
            .output_bytes(id)
            .ok_or(engine::control::Error::Runtime(
                engine::RuntimeError::JobNotRetained,
            ))?;
        let mut base_rev = None;
        let (command, params) = match task {
            Task::Generate {
                start,
                auto,
                replace,
            } => {
                let result = engine::subtitles::decode_captions(bytes)
                    .map_err(engine::control::Error::from)?;
                // The detected tag is checked against the languages this app's ASR supports.
                if auto && !SOURCE_LANGUAGES.contains(&result.source_language.as_str()) {
                    return Err(engine::transcript::Error::LowConfidence.into());
                }
                // A target the user removed meanwhile is dropped, not refused: the generated
                // captions still apply.
                let replace = replace.filter(|old| {
                    session
                        .project()
                        .data()
                        .scene()
                        .timeline
                        .iter()
                        .flat_map(|track| &track.items)
                        .any(|item| {
                            item.id == *old && matches!(item.content, ItemParams::Caption(_))
                        })
                });
                (
                    "captions.apply",
                    serde_json::json!({"job": id, "start": start, "replace": replace}),
                )
            }
            Task::Translate { revision } => {
                let result = engine::subtitles::decode_translations(bytes)
                    .map_err(engine::control::Error::from)?;
                base_rev = Some(revision);
                (
                    "retranslate",
                    serde_json::json!({
                        "item": result.item,
                        "target_language": result.target_language,
                        "texts": result.texts,
                        "translation_ref": result.translation_ref,
                        "force": self.force,
                    }),
                )
            }
            Task::Import { .. } => {
                return Err(engine::control::Error::Edit(engine::EditError::Invalid(
                    "subtitle import is not a registry job",
                )));
            }
        };
        match ai_access::run(session, &mut self.runner, None, base_rev, command, params)? {
            engine::control::ControlResult::Edit { outcome } => Ok(outcome.created),
            _ => Err(engine::control::Error::Edit(engine::EditError::Invalid(
                "subtitle result was not applied",
            ))),
        }
    }
    fn fail(&mut self, error: Arc<dyn engine::Failure + 'static>) {
        self.error = Some(error.clone());
        let cause: &(dyn std::error::Error + 'static) = error.as_ref();
        let root = cause.downcast_ref::<engine::control::Error>();
        let subtitles = match root {
            Some(engine::control::Error::Subtitles(subtitles)) => Some(subtitles),
            _ => cause.downcast_ref::<engine::subtitles::Error>(),
        };
        let transcript = match subtitles {
            Some(engine::subtitles::Error::Transcript(transcript)) => Some(transcript),
            _ => match root {
                Some(engine::control::Error::Transcript(transcript)) => Some(transcript),
                _ => cause.downcast_ref::<engine::transcript::Error>(),
            },
        };
        let runtime = match subtitles {
            Some(engine::subtitles::Error::Runtime(error)) => Some(error),
            _ => match transcript {
                Some(engine::transcript::Error::Runtime(error)) => Some(error),
                _ => match root {
                    Some(engine::control::Error::Runtime(error)) => Some(error),
                    _ => cause.downcast_ref::<engine::RuntimeError>(),
                },
            },
        };
        if let Some(engine::RuntimeError::Failed(inner)) = runtime {
            self.fail(inner.clone());
            return;
        }
        if matches!(runtime, Some(engine::RuntimeError::Cancelled)) {
            self.problem = Some(Text::ShortsJobCancelled);
            return;
        }
        if matches!(
            root,
            Some(engine::control::Error::Control(
                engine::control::ControlError::StaleRevision
            ))
        ) || matches!(
            subtitles,
            Some(engine::subtitles::Error::Edit(engine::EditError::Session(
                engine::SessionError::StaleResult
            )))
        ) || matches!(
            transcript,
            Some(engine::transcript::Error::Edit(engine::EditError::Session(
                engine::SessionError::StaleResult
            )))
        ) || matches!(
            runtime,
            Some(engine::RuntimeError::Edit(engine::EditError::Session(
                engine::SessionError::StaleResult
            )))
        ) || matches!(
            cause.downcast_ref::<engine::EditError>(),
            Some(engine::EditError::Session(
                engine::SessionError::StaleResult
            ))
        ) {
            self.problem = Some(Text::SubStale);
            return;
        }
        match transcript {
            Some(engine::transcript::Error::NoSpeech) => {
                self.problem = Some(Text::SubNoSpeech);
                return;
            }
            Some(engine::transcript::Error::LowConfidence) => {
                self.problem = Some(Text::SubLowConfidence);
                return;
            }
            Some(engine::transcript::Error::Runtime(engine::RuntimeError::Cancelled)) => {
                self.problem = Some(Text::ShortsJobCancelled);
                return;
            }
            _ => {}
        }
        let provider = match subtitles {
            Some(engine::subtitles::Error::Providers(provider)) => Some(provider),
            _ => match transcript {
                Some(engine::transcript::Error::Providers(provider)) => Some(provider),
                _ => match root {
                    Some(engine::control::Error::Providers(provider)) => Some(provider),
                    _ => cause.downcast_ref::<engine::providers::Error>(),
                },
            },
        };
        if let Some(provider) = provider {
            use engine::providers::Error as P;
            self.problem = Some(match provider {
                P::ModelMissing(id) => {
                    self.missing_model = Some((*id).into());
                    Text::ShortsModelMissing
                }
                P::CredentialMissing(_) => Text::ShortsCredentialsMissing,
                P::RunnerMissing => Text::SubRunnerMissing,
                P::AutoLanguageUnsupported => Text::SubAutoUnsupported,
                P::ConsentRequired => Text::ShortsConsentRequired,
                P::Runtime(engine::RuntimeError::Cancelled) => Text::ShortsJobCancelled,
                _ => Text::SubFailed,
            });
            return;
        }
        self.problem = Some(match root {
            Some(engine::control::Error::Session(engine::SessionError::StaleResult))
            | Some(engine::control::Error::Edit(engine::EditError::Session(
                engine::SessionError::StaleResult,
            ))) => Text::SubStale,
            Some(engine::control::Error::Runtime(engine::RuntimeError::Cancelled)) => {
                Text::ShortsJobCancelled
            }
            _ => Text::SubFailed,
        });
    }
    fn caption<'a>(
        &self,
        session: &'a Session,
    ) -> Option<(&'a project::Item, &'a project::CaptionParams)> {
        session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find_map(|i| {
                if Some(i.id) == self.item
                    && let ItemParams::Caption(p) = &i.content
                {
                    Some((i, p))
                } else {
                    None
                }
            })
    }
    fn refresh(&mut self, session: &Session) {
        let previous = self.item;
        if self.caption(session).is_none() {
            self.item = session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|t| &t.items)
                .find(|i| matches!(i.content, ItemParams::Caption(_)))
                .map(|i| i.id);
        }
        if previous != self.item
            && self.drafts.iter().any(|((id, language), draft)| {
                self.draft_project
                    .as_ref()
                    .and_then(|p| draft_text(p, previous, id, language))
                    .map_or(!draft.is_empty(), |text| text != draft)
            })
        {
            self.item = previous;
            self.problem = Some(Text::SubDraftConflict);
            return;
        }
        if previous != self.item {
            self.selected.clear();
            self.anchor = None;
            self.page = 0;
            self.drafts.clear();
        }
        if self
            .draft_project
            .as_ref()
            .is_none_or(|p| !Arc::ptr_eq(p, session.project()))
        {
            let original = self.draft_project.clone();
            let item = self.item;
            self.drafts.retain(|(id, language), draft| {
                let text = original
                    .as_ref()
                    .and_then(|p| draft_text(p, item, id, language));
                text.map_or(!draft.is_empty(), |text| text != draft)
            });
            let conflict = self.drafts.keys().any(|(id, language)| {
                original
                    .as_ref()
                    .and_then(|p| draft_text(p, item, id, language))
                    != draft_text(session.project(), item, id, language)
            });
            if conflict {
                self.problem = Some(Text::SubDraftConflict);
                return;
            }
            self.draft_project = Some(session.project().clone());
            let ids: BTreeSet<_> = self
                .caption(session)
                .map(|(_, p)| p.cues.cues.iter().map(|c| c.id.clone()).collect())
                .unwrap_or_default();
            self.selected.retain(|id| ids.contains(id));
            if self.anchor.as_ref().is_some_and(|id| !ids.contains(id)) {
                self.anchor = self.caption(session).and_then(|(_, p)| {
                    p.cues
                        .cues
                        .iter()
                        .find(|cue| self.selected.contains(&cue.id))
                        .map(|cue| cue.id.clone())
                });
            }
        }
    }
}
/// The presentation stage of an engine job stage; anything else shows as loading.
/// The captions job runner: one job per class, a short queue and one network attempt.
fn job_runner(budget: &ByteBudget) -> Result<JobRunner> {
    Ok(JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 4,
            retained: 8,
            network_attempts: 1,
            backoff: Duration::from_millis(10),
            progress_interval: Duration::from_millis(20),
        },
        budget.clone(),
        Arc::new(SystemClock::default()),
    )?)
}
fn stage_of(stage: JobStage) -> Option<Stage> {
    match stage {
        JobStage::Extract => Some(Stage::Extract),
        JobStage::Transcribe => Some(Stage::Transcribe),
        JobStage::Align => Some(Stage::Align),
        JobStage::Segment => Some(Stage::Segment),
        JobStage::Assemble => Some(Stage::Assemble),
        JobStage::Translate => Some(Stage::Translate),
        _ => None,
    }
}
fn terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Cancelled | JobState::Failed
    )
}
fn key() -> engine::control::Result<CacheKey> {
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::Cues,
        source: [0; 32],
        provider: "desktop",
        model: "subtitles",
        version: "1",
        settings: &[],
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .map_err(engine::control::Error::from)
}

fn draft_text<'a>(
    project: &'a project::Project,
    item: Option<ItemId>,
    cue: &NfcText,
    language: &LanguageTag,
) -> Option<&'a str> {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find_map(|i| {
            if Some(i.id) == item
                && let ItemParams::Caption(p) = &i.content
            {
                p.cues
                    .cues
                    .iter()
                    .find(|c| &c.id == cue)
                    .and_then(|c| c.texts.get(language))
                    .map(|t| t.text.as_str())
            } else {
                None
            }
        })
}
