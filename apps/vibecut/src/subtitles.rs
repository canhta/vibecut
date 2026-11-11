//! Captions panel: presentation and bounded staging through existing engine owners.
mod editing;
mod view;
mod workflow;
pub use crate::services::ai_task::TaskEvents;
use crate::{Error, Result, egui, i18n::Text};
use captions::{LanguageTag, NfcText};
use engine::{
    ByteBudget, Command, Session,
    cache::{ArtifactKind, CacheKey, KeyInputs},
    jobs::{CancelToken, JobId, JobKind, JobRunner, JobSpec, JobState, RunnerConfig, SystemClock},
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
pub use workflow::LocalWorkflow;

/// Persisted explicit choices. Consent, glossary and private text are never preferences.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// `auto` or one of the spoken languages this app's ASR supports.
    pub source: String,
    pub target: String,
    pub transcription: String,
    pub transcription_model: String,
    pub translation: String,
    pub translation_model: String,
    pub bilingual: bool,
    /// Last spoken-language choice per project; absent means Auto.
    pub project_sources: BTreeMap<project::ProjectId, String>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            source: "auto".into(),
            target: "en".into(),
            transcription: "whisper.cpp".into(),
            transcription_model: "whisper-large-v3-turbo".into(),
            translation: "openai".into(),
            translation_model: ai::openai::TRANSLATION_MODEL.into(),
            bilingual: false,
            project_sources: BTreeMap::new(),
        }
    }
}
/// Every spoken language the configured ASR adapters actually accept.
pub const SOURCE_LANGUAGES: [&str; 3] = ["vi", "en", "zh"];
impl Options {
    pub fn validate(&self) -> Result<()> {
        if (self.source != "auto" && !SOURCE_LANGUAGES.contains(&self.source.as_str()))
            || !["vi", "en", "zh-Hans", "zh-Hant"].contains(&self.target.as_str())
        {
            return Err(Error::Invalid("subtitle language"));
        }
        if self.project_sources.len() > 1024
            || self
                .project_sources
                .values()
                .any(|source| source != "auto" && !SOURCE_LANGUAGES.contains(&source.as_str()))
        {
            return Err(Error::Invalid("subtitle project language"));
        }
        engine::providers::choose_transcription(
            &self.transcription,
            Some(&self.transcription_model),
        )?;
        engine::providers::choose_translation(&self.translation, Some(&self.translation_model))?;
        Ok(())
    }
    /// Auto is the default until a project stores an explicit choice.
    pub fn source_for(&self, project: project::ProjectId) -> &str {
        self.project_sources
            .get(&project)
            .map_or("auto", String::as_str)
    }
    pub fn load_source(&mut self, project: project::ProjectId) {
        self.source = self.source_for(project).to_owned();
    }
    pub fn remember_source(&mut self, project: project::ProjectId) {
        if self.source_for(project) != self.source {
            self.project_sources.insert(project, self.source.clone());
        }
    }
    /// Provider filtering sees Auto as "no language filter", never as a tag.
    pub(crate) fn source_tag(&self) -> Option<LanguageTag> {
        (self.source != "auto")
            .then(|| LanguageTag::parse(&self.source).ok())
            .flatten()
    }
    pub(crate) fn transcription_supported(&self) -> bool {
        self.validate().is_ok()
            && engine::providers::provider_options_for(
                engine::providers::CatalogOperation::Transcribe,
                self.source_tag().as_ref(),
            )
            .iter()
            .any(|p| p.id == self.transcription && p.models.contains(&self.transcription_model))
    }
}
/// Immutable input to a real engine workflow; no UI state is shared with a worker.
#[derive(Clone)]
pub enum Work {
    Generate {
        asset: media::SourceAsset,
        range: time::SourceRange,
        start: Tick,
        options: Options,
    },
    Import {
        asset_id: media::AssetId,
        path: PathBuf,
        language: LanguageTag,
        start: Tick,
    },
    Translate {
        project: Arc<project::Project>,
        item: ItemId,
        ids: Vec<NfcText>,
        options: Options,
        glossary: Vec<ai::GlossaryTerm>,
        force: bool,
    },
}
/// Injectable only at the blocking composition boundary. Engine commands still apply
/// the returned batch against the original Session revision, atomically.
pub trait Workflow: Send + Sync + 'static {
    fn run(
        &self,
        work: Work,
        budget: ByteBudget,
        events: &TaskEvents,
        cancel: &Cancellation,
    ) -> engine::Result<Command>;
}
struct Active {
    id: JobId,
    token: CancelToken,
    cancel: Cancellation,
    events: Arc<TaskEvents>,
    receive: mpsc::Receiver<std::result::Result<Command, Arc<engine::EditError>>>,
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
}
#[derive(Clone, Copy)]
enum Pick {
    Import,
    Srt,
    Vtt,
    Txt,
    Burn,
}
struct Picker {
    kind: Pick,
    future: std::pin::Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>>,
    waker: std::task::Waker,
}
struct Repaint(egui::Context);
impl std::task::Wake for Repaint {
    fn wake(self: Arc<Self>) {
        self.0.request_repaint();
    }
}
/// Panel state survives tab and locale changes; only Options enter preferences.
pub struct Subtitles {
    workflow: Arc<dyn Workflow>,
    budget: ByteBudget,
    active: Option<Active>,
    active_options: Option<Options>,
    export: Option<(JobId, CancelToken)>,
    export_count: Option<usize>,
    runner: JobRunner,
    pub(crate) item: Option<ItemId>,
    selected: BTreeSet<NfcText>,
    anchor: Option<NfcText>,
    page: usize,
    text_focus: Option<egui::Id>,
    drafts: BTreeMap<(NfcText, LanguageTag), String>,
    draft_project: Option<Arc<project::Project>>,
    pub glossary: Vec<ai::GlossaryTerm>,
    force: bool,
    problem: Option<Text>,
    error: Option<Arc<dyn engine::Failure + 'static>>,
    missing_model: Option<String>,
    completed: Option<usize>,
    pub(crate) notice: Option<usize>,
    picker: Option<Picker>,
    overwrite: Option<(PathBuf, Pick)>,
    pub(crate) requested_model: Option<String>,
    pub(crate) requested_seek: Option<Tick>,
    project: Option<project::ProjectId>,
    retry_source: Option<ItemId>,
}
impl Drop for Subtitles {
    fn drop(&mut self) {
        self.cancel();
        self.runner.shutdown();
    }
}
impl Subtitles {
    pub fn new(budget: ByteBudget) -> Result<Self> {
        Self::with_workflow(budget, Arc::new(LocalWorkflow))
    }
    pub fn with_workflow(budget: ByteBudget, workflow: Arc<dyn Workflow>) -> Result<Self> {
        let runner = JobRunner::new(
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
        )?;
        Ok(Self {
            workflow,
            budget,
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
            drafts: BTreeMap::new(),
            draft_project: None,
            glossary: Vec::new(),
            force: false,
            problem: None,
            error: None,
            missing_model: None,
            completed: None,
            notice: None,
            picker: None,
            overwrite: None,
            requested_model: None,
            requested_seek: None,
            project: None,
            retry_source: None,
        })
    }
    pub fn busy(&self) -> bool {
        self.active.is_some() || self.export.is_some()
    }
    pub fn inspect(&self, session: &Session) -> Inspection {
        let state = self
            .active
            .as_ref()
            .and_then(|a| a.events.state.lock().ok());
        Inspection {
            busy: self.busy(),
            job_state: self
                .active
                .as_ref()
                .and_then(|a| self.runner.snapshot(a.id))
                .or_else(|| {
                    self.export
                        .as_ref()
                        .and_then(|(id, _)| self.runner.snapshot(*id))
                })
                .map(|s| s.state),
            cancelling: self.active.as_ref().is_some_and(|a| a.cancel.cancelled()),
            stage: state.as_ref().and_then(|s| s.stage),
            progress: state.as_ref().and_then(|s| s.fraction).or_else(|| {
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
        }
    }
    pub(crate) fn editing(&self, ctx: &egui::Context) -> bool {
        self.text_focus
            .is_some_and(|id| ctx.memory(|m| m.focused()) == Some(id))
    }
    pub(crate) fn modal_open(&self) -> bool {
        self.overwrite.is_some()
            || self
                .active
                .as_ref()
                .is_some_and(|a| a.events.state.lock().is_ok_and(|s| s.upload.is_some()))
    }
    pub(crate) fn invalidate_options(&mut self, options: &Options) {
        if self.active.is_some() && self.active_options.as_ref().is_some_and(|o| o != options) {
            self.cancel();
        }
    }
    pub fn cancel(&mut self) {
        if let Some(active) = &self.active {
            active.cancel.cancel();
            active.token.cancel();
        }
        if let Some((_, token)) = &self.export {
            token.cancel();
        }
    }
    pub fn approve_upload(&mut self, approved: bool) -> Result<()> {
        let active = self
            .active
            .as_ref()
            .ok_or(Error::Invalid("no subtitle job"))?;
        if active.cancel.cancelled() {
            return Err(Error::Cancelled);
        }
        let upload = active
            .events
            .state
            .lock()
            .map_err(|_| Error::Poisoned)?
            .upload
            .take()
            .ok_or(Error::Invalid("no subtitle upload"))?;
        upload.reply.send(approved).map_err(|_| Error::Cancelled)
    }
    fn start(&mut self, session: &Session, work: Work) -> Result<()> {
        if self.busy() || session.transaction_active() {
            return Err(Error::Invalid("subtitle job busy"));
        }
        let cancel = Cancellation::new();
        let events = Arc::new(TaskEvents::new(cancel.clone()));
        let (send, receive) = mpsc::sync_channel(1);
        let worker = self.workflow.clone();
        let budget = self.budget.clone();
        let worker_events = events.clone();
        let worker_cancel = cancel.clone();
        self.active_options = match &work {
            Work::Generate { options, .. } | Work::Translate { options, .. } => {
                Some(options.clone())
            }
            _ => None,
        };
        let kind = if matches!(&work, Work::Translate { .. }) {
            JobKind::Translate
        } else {
            JobKind::Transcribe
        };
        let (id, token) = self.runner.submit_for_session(
            session,
            JobSpec {
                kind,
                class: engine::jobs::ResourceClass::Cpu,
                input: key()?,
                bytes: engine::jobs::JobBytes {
                    result: 32 * 1024 * 1024,
                    ..Default::default()
                },
            },
            move |ctx| {
                ctx.check_cancelled()?;
                let result =
                    worker.run(work.clone(), budget.clone(), &worker_events, &worker_cancel);
                match result {
                    Ok(command) => {
                        send.send(Ok(command))
                            .map_err(|_| engine::EditError::Cancelled)?;
                        Ok(Vec::new())
                    }
                    Err(error) => {
                        let error = Arc::new(error);
                        send.send(Err(error.clone()))
                            .map_err(|_| engine::EditError::Cancelled)?;
                        Err(engine::EditError::Runtime(engine::RuntimeError::Failed(
                            error,
                        )))
                    }
                }
            },
        )?;
        self.active = Some(Active {
            id,
            token,
            cancel,
            events,
            receive,
        });
        self.problem = None;
        self.error = None;
        self.completed = None;
        self.missing_model = None;
        Ok(())
    }
    pub fn generate(&mut self, session: &Session, source: ItemId, options: &Options) -> Result<()> {
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
        if !item.retime.speed.is_unity() {
            return Err(Error::Invalid("captions for a retimed item"));
        }
        let asset = session
            .project()
            .data()
            .assets
            .iter()
            .find(|a| Some(a.id()) == item.source_asset)
            .ok_or(Error::Invalid("select source media"))?
            .clone();
        if !asset
            .probe()
            .streams
            .iter()
            .any(|s| s.kind == media::StreamKind::Audio)
        {
            return Err(Error::Invalid("source has no audio"));
        }
        self.start(
            session,
            Work::Generate {
                asset,
                range: item.source_range,
                start: item.start,
                options: options.clone(),
            },
        )
    }
    /// Re-run the last source with auto-detect after a no-speech/low-confidence
    /// result. The caller's Options keep the new choice for this project.
    pub fn retry_auto(&mut self, session: &Session, options: &mut Options) -> Result<()> {
        let source = self.retry_source.ok_or(Error::Invalid("no source media"))?;
        options.source = "auto".into();
        if let Some(project) = self.project {
            options.remember_source(project);
        }
        self.generate(session, source, options)
    }
    pub fn import_file(
        &mut self,
        session: &Session,
        path: PathBuf,
        source: &str,
        at: Tick,
    ) -> Result<()> {
        self.start(
            session,
            Work::Import {
                asset_id: media::AssetId(
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
                ),
                path,
                language: LanguageTag::parse(source)?,
                start: at,
            },
        )
    }
    pub fn translate(&mut self, session: &Session, options: &Options) -> Result<()> {
        engine::providers::choose_translation(
            &options.translation,
            Some(&options.translation_model),
        )?;
        let item = self.item.ok_or(Error::Invalid("select captions"))?;
        if self.selected.is_empty() {
            return Err(Error::Invalid("select cues"));
        }
        self.start(
            session,
            Work::Translate {
                project: session.project().clone(),
                item,
                ids: self.selected.iter().cloned().collect(),
                options: options.clone(),
                glossary: self.glossary.clone(),
                force: self.force,
            },
        )
    }
    /// Poll even while another tab is visible. Terminal results never apply after cancellation.
    pub fn tick(&mut self, session: &mut Session) -> Result<bool> {
        self.runner.tick()?;
        let mut changed = false;
        if self.active.as_ref().is_some_and(|a| {
            self.runner
                .snapshot(a.id)
                .is_some_and(|s| terminal(s.state))
        }) {
            let active = self.active.take().ok_or(Error::Invalid("subtitle job"))?;
            let output = self.runner.take_output(active.id);
            if active.cancel.cancelled() {
                self.problem = Some(Text::ShortsJobCancelled);
            } else {
                let result = active.receive.try_recv().unwrap_or_else(|_| {
                    Err(Arc::new(engine::EditError::Invalid(
                        "subtitle worker terminated",
                    )))
                });
                match result {
                    Ok(command) => {
                        let output = output.ok_or(Error::Invalid("subtitle result missing"))?;
                        match output.apply(session, command) {
                            Ok(outcome) => {
                                self.item = outcome
                                    .created
                                    .iter()
                                    .find_map(|id| {
                                        if let engine::CreatedId::Item(id) = id {
                                            Some(*id)
                                        } else {
                                            None
                                        }
                                    })
                                    .or(self.item);
                                self.completed = Some(
                                    self.caption(session).map_or(0, |(_, p)| p.cues.cues.len()),
                                );
                                self.notice = self.completed;
                                changed = true;
                            }
                            Err(e) => self.fail(Arc::new(e)),
                        }
                    }
                    Err(e) => self.fail(e),
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
    fn fail(&mut self, error: Arc<dyn engine::Failure + 'static>) {
        self.error = Some(error.clone());
        let cause: &(dyn std::error::Error + 'static) = error.as_ref();
        let root = cause.downcast_ref::<engine::EditError>();
        let runtime = match root {
            Some(engine::EditError::Runtime(error)) => Some(error),
            _ => cause.downcast_ref::<engine::RuntimeError>(),
        };
        if let Some(engine::RuntimeError::Failed(inner)) = runtime {
            self.fail(inner.clone());
            return;
        }
        self.problem = Some(match root {
            Some(engine::EditError::Providers(engine::providers::Error::ModelMissing(id))) => {
                self.missing_model = Some((*id).into());
                Text::ShortsModelMissing
            }
            Some(engine::EditError::Providers(engine::providers::Error::CredentialMissing(_))) => {
                Text::ShortsCredentialsMissing
            }
            Some(engine::EditError::Providers(engine::providers::Error::RunnerMissing)) => {
                Text::SubRunnerMissing
            }
            Some(engine::EditError::Providers(
                engine::providers::Error::AutoLanguageUnsupported,
            )) => Text::SubAutoUnsupported,
            Some(engine::EditError::Providers(engine::providers::Error::ConsentRequired)) => {
                Text::ShortsConsentRequired
            }
            Some(engine::EditError::Transcript(engine::transcript::Error::NoSpeech)) => {
                Text::SubNoSpeech
            }
            Some(engine::EditError::Transcript(engine::transcript::Error::LowConfidence)) => {
                Text::SubLowConfidence
            }
            Some(engine::EditError::StaleResult) => Text::SubStale,
            Some(engine::EditError::Cancelled) => Text::ShortsJobCancelled,
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
fn terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Cancelled | JobState::Failed
    )
}
fn key() -> engine::Result<CacheKey> {
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
