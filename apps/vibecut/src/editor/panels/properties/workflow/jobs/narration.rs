//! Runs one narration for the Text to speech tab: the provider form, the `dub.generate` job
//! that makes the take, the fit decision and the placement of the take. The panel submits the
//! same registered command an agent or the CLI does; the take becomes an item through
//! `dub.place`, and the panel never writes project data itself.
use crate::editor::properties::{State, read};
use crate::editor::{Command, CreatedId, Error, ItemId, Port, Result, SourceAsset, Tick};
use crate::services::{ai_access, ai_task::TaskEvents};
use captions::{LanguageTag, NfcText};
use engine::Session;
use engine::control::{Caller, ControlResult, ErrorCode};
use engine::dubbing::{
    CredentialSpeech, DubTake, DurationFit, Fit, FitInput, SpeechOpener, SpeechOption, decode_take,
    fit, speech_options, write_wav,
};
use engine::jobs::{JobId, JobRunner, JobState};
use engine::providers::{Credentials, UploadRequest};
use engine::subtitles::CaptionAccess;
use engine::transcript::Cancellation;
use media::AssetId;
use project::{ItemParams, NarrationDucking};
use std::{path::PathBuf, sync::Arc};

/// The spoken languages the speech adapters take. No other tag is offered.
pub(in crate::editor) const LANGUAGES: [&str; 3] = ["vi", "en", "zh"];

/// What the user chose in the form: indices into the provider list, the provider's models and
/// voices, and [`LANGUAGES`]. Nothing is chosen for the user and nothing falls back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) struct Form {
    pub(in crate::editor) provider: usize,
    pub(in crate::editor) model: usize,
    pub(in crate::editor) voice: usize,
    pub(in crate::editor) language: usize,
}

/// Why a provider run ended without a take, from the typed AI error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Cause {
    Language,
    Voice,
    Key,
    Quota,
    Policy,
    Rejected,
    Cost,
    Network,
    Cancelled,
    Other,
}

/// Why the last generation or placement left the project untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Failed {
    Provider(Cause),
    /// The person declined the upload; nothing was sent.
    Declined,
    EmptyText,
    /// The text item changed while the take was made.
    Stale,
    Storage,
    Placement,
}

/// One finished take as the tab shows it before placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::editor) struct TakeView {
    pub(in crate::editor) natural: Tick,
    pub(in crate::editor) cue: Tick,
    pub(in crate::editor) fit: std::result::Result<Fit, DurationFit>,
    pub(in crate::editor) ducking: NarrationDucking,
    pub(in crate::editor) provider_speed: Option<u16>,
}

/// What the tab asks the workflow to do.
#[derive(Clone, Copy, Debug)]
pub(in crate::editor) enum Action {
    /// Prepare the request, ask for consent and synthesize.
    Generate {
        item: ItemId,
    },
    /// Generate again at the shown provider speed (thousandths).
    Regenerate {
        item: ItemId,
        speed: u16,
    },
    /// The consent sheet's answer.
    Consent(bool),
    Cancel,
    /// Place the take through `dub.place`.
    Place {
        item: ItemId,
    },
    Discard,
    /// Ducking of the pending take, or of the placed one through `dub.set_ducking`.
    SetDucking(NarrationDucking),
}

/// The running `dub.generate` job and the consent slot its prompt waits on.
struct Run {
    events: Arc<TaskEvents>,
    cancel: Cancellation,
    job: JobId,
    item: ItemId,
    cue: Tick,
}
impl Drop for Run {
    /// A run that leaves the tab stops the consent wait its job may be blocked in.
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// The desktop is the trusted root of its own session: consent, not policy, guards the call.
struct Human;
impl engine::control::Policy for Human {
    fn authorize(&self, _: &engine::Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

struct Ready {
    take: DubTake,
    item: ItemId,
    cue: Tick,
    fit: std::result::Result<Fit, DurationFit>,
}

pub(in crate::editor) struct NarrationWorkflow {
    options: Vec<SpeechOption>,
    pub(in crate::editor) form: Form,
    opener: Option<Arc<dyn SpeechOpener>>,
    directory: Option<PathBuf>,
    /// The text to read, editable, bound to the text item it was read from.
    pub(in crate::editor) text: Option<(ItemId, String)>,
    /// The runner `dub.generate` is submitted to; created on first use.
    runner: Option<JobRunner>,
    run: Option<Run>,
    /// A job whose run was dropped (another item was selected); `dub.cancel` stops it.
    orphan: Option<JobId>,
    ready: Option<Ready>,
    placed: Option<ItemId>,
    failure: Option<Failed>,
    /// A shorter text was already asked for, so the next fit may overflow.
    rephrased: bool,
    /// Ducking values being edited; committed when the field is released.
    pub(in crate::editor) ducking_draft: Option<NarrationDucking>,
}

impl Default for NarrationWorkflow {
    fn default() -> Self {
        Self {
            options: speech_options(),
            form: Form {
                provider: 0,
                model: 0,
                voice: 0,
                language: 1,
            },
            opener: None,
            directory: None,
            text: None,
            runner: None,
            run: None,
            orphan: None,
            ready: None,
            placed: None,
            failure: None,
            rephrased: false,
            ducking_draft: None,
        }
    }
}

/// An AI failure as the creator-facing cause.
fn cause_of(error: &ai::Error) -> Cause {
    match error {
        ai::Error::UnsupportedLanguage => Cause::Language,
        ai::Error::UnsupportedVoice => Cause::Voice,
        ai::Error::Auth => Cause::Key,
        ai::Error::Quota | ai::Error::RateLimit { .. } => Cause::Quota,
        ai::Error::ContentPolicy | ai::Error::Refused => Cause::Policy,
        ai::Error::OutputRejected | ai::Error::Incomplete => Cause::Rejected,
        ai::Error::Cost => Cause::Cost,
        ai::Error::Network | ai::Error::Timeout | ai::Error::Server => Cause::Network,
        ai::Error::Cancelled => Cause::Cancelled,
        _ => Cause::Other,
    }
}

/// A provider-layer refusal (preparation or consent) as the failure line.
fn failed_of(error: &engine::providers::Error) -> Failed {
    use engine::providers::Error as Refusal;
    match error {
        Refusal::ConsentRequired => Failed::Declined,
        Refusal::Ai(error) => Failed::Provider(cause_of(error)),
        Refusal::CredentialMissing(_) => Failed::Provider(Cause::Key),
        Refusal::Runtime(engine::RuntimeError::Cancelled) => Failed::Provider(Cause::Cancelled),
        _ => Failed::Provider(Cause::Other),
    }
}

/// A refusal of `dub.generate` itself: language, voice, key and cue are checked before the job.
fn failed_of_control(error: &engine::control::Error) -> Failed {
    use engine::control::Error as Refusal;
    match error {
        Refusal::Providers(error) => failed_of(error),
        Refusal::Ai(error) => Failed::Provider(cause_of(error)),
        Refusal::Runtime(engine::RuntimeError::Cancelled) => Failed::Provider(Cause::Cancelled),
        _ => Failed::Provider(Cause::Other),
    }
}

/// How a failed job reads: the runner keeps its wire code, which is coarser than the AI cause.
fn failed_of_code(code: ErrorCode) -> Failed {
    match code {
        ErrorCode::Permission => Failed::Declined,
        ErrorCode::Cancelled => Failed::Provider(Cause::Cancelled),
        ErrorCode::Setup | ErrorCode::Auth => Failed::Provider(Cause::Key),
        ErrorCode::Quota | ErrorCode::RateLimit => Failed::Provider(Cause::Quota),
        ErrorCode::Network => Failed::Provider(Cause::Network),
        _ => Failed::Provider(Cause::Other),
    }
}

/// Silent time after `cue_end` before the next audible item starts: the room an overflow may
/// use. Matches the engine's rule that no unmuted audio or video with sound may overlap it.
fn silent_after(project: &project::Project, cue_end: Tick) -> Tick {
    let mut room = i64::MAX / 4;
    for track in project.data().scene().timeline.iter() {
        for other in &track.items {
            let audible = match &other.content {
                ItemParams::Audio(params) => !params.muted,
                ItemParams::Video(params) => params.source_audio_enabled && !params.muted,
                _ => false,
            };
            if audible && other.end().is_ok_and(|end| end > cue_end) {
                room = room.min(other.start.0.saturating_sub(cue_end.0).max(0));
            }
        }
    }
    Tick(room)
}

/// The text item's content, start and duration.
fn text_facts(project: &project::Project, item: ItemId) -> Option<(String, Tick, Tick)> {
    let found = read::item(project, item)?;
    let ItemParams::Text(params) = &found.content else {
        return None;
    };
    Some((
        params.content.as_str().to_owned(),
        found.start,
        found.duration().ok()?,
    ))
}

impl NarrationWorkflow {
    /// Inject the speech opener and the folder takes are written to.
    pub(in crate::editor) fn set(&mut self, opener: Arc<dyn SpeechOpener>, directory: PathBuf) {
        self.opener = Some(opener);
        self.directory = Some(directory);
    }

    pub(in crate::editor) fn options(&self) -> &[SpeechOption] {
        &self.options
    }

    /// The provider the form currently names.
    pub(in crate::editor) fn option(&self) -> Option<&SpeechOption> {
        self.options.get(self.form.provider)
    }

    /// Bind the form's text to the selected text item. A different item drops the take and
    /// any run of the previous one; the same item keeps the typed text.
    pub(in crate::editor) fn bind(&mut self, item: ItemId, content: &str) {
        if self.text.as_ref().is_some_and(|(bound, _)| *bound == item) {
            return;
        }
        if let Some(run) = self.run.take() {
            self.orphan = Some(run.job);
        }
        self.ready = None;
        self.placed = None;
        self.failure = None;
        self.rephrased = false;
        self.ducking_draft = None;
        self.text = Some((item, content.to_owned()));
    }

    /// Submit one registered command to this workflow's runner as the desktop person.
    fn control(
        &mut self,
        session: &mut Session,
        budget: &engine::ByteBudget,
        command: engine::Command,
        access: Option<&CaptionAccess>,
    ) -> engine::control::Result<ControlResult> {
        let runner = match self.runner.take() {
            Some(runner) => runner,
            None => JobRunner::new(
                engine::jobs::RunnerConfig {
                    concurrency: [1, 1, 1],
                    queue: 1,
                    retained: 4,
                    network_attempts: 1,
                    backoff: std::time::Duration::from_millis(100),
                    progress_interval: std::time::Duration::from_millis(40),
                },
                budget.clone(),
                Arc::new(engine::jobs::SystemClock::default()),
            )?,
        };
        let runner = self.runner.insert(runner);
        session.control_command(
            command,
            Caller {
                principal: "vibecut-desktop",
                external: false,
                policy: &Human,
                fonts: None,
                sounds: None,
                music: None,
                stickers: None,
                brand: None,
                media: None,
                chapters: None,
                captions: access,
                vision: None,
            },
            runner,
        )
    }

    pub(in crate::editor) fn busy(&self) -> bool {
        self.run.is_some()
    }

    /// The upload the consent sheet shows, while the worker waits for the answer.
    pub(in crate::editor) fn pending(&self) -> Option<(UploadRequest, bool)> {
        let run = self.run.as_ref()?;
        let state = run.events.state.lock().ok()?;
        state
            .upload
            .as_ref()
            .map(|upload| (upload.request.clone(), upload.price_acknowledged))
    }

    /// Tick the price acknowledgement of the pending unknown-price sheet.
    pub(in crate::editor) fn acknowledge_price(&self, acknowledged: bool) {
        if let Some(run) = &self.run
            && let Ok(mut state) = run.events.state.lock()
            && let Some(upload) = &mut state.upload
        {
            upload.price_acknowledged = acknowledged;
        }
    }

    pub(in crate::editor) fn failure(&self) -> Option<Failed> {
        self.failure
    }

    pub(in crate::editor) fn placed(&self) -> Option<ItemId> {
        self.placed
    }

    pub(in crate::editor) fn take(&self) -> Option<TakeView> {
        let ready = self.ready.as_ref()?;
        Some(TakeView {
            natural: ready.take.natural().ok()?,
            cue: ready.cue,
            fit: ready.fit,
            ducking: ready.take.narration.ducking,
            provider_speed: ready.take.narration.provider_speed_thousandths,
        })
    }

    fn discard(&mut self) {
        self.ready = None;
        self.rephrased = false;
        self.failure = None;
        self.ducking_draft = None;
    }
}

impl State {
    /// Inject the speech opener and the take folder, for composition and tests.
    pub(in crate::editor) fn set_narration(
        &mut self,
        opener: Arc<dyn SpeechOpener>,
        directory: PathBuf,
    ) {
        self.narration.set(opener, directory);
    }

    /// Whether the narration consent sheet is open and owns the keyboard.
    pub(in crate::editor) fn narration_modal(&self) -> bool {
        self.narration.pending().is_some()
    }

    /// Carry out one request of the tab.
    pub(in crate::editor) fn narration_action(
        &mut self,
        port: &mut Port<'_>,
        action: Action,
    ) -> Result<()> {
        match action {
            Action::Generate { item } => self.start_narration(port, item, None),
            Action::Regenerate { item, speed } => self.start_narration(port, item, Some(speed)),
            Action::Consent(approved) => match &self.narration.run {
                Some(run) => run.events.answer(approved),
                None => Ok(()),
            },
            Action::Cancel => {
                if let Some(run) = self.narration.run.take() {
                    // The same `dub.cancel` an agent sends, and the consent wait it may be in.
                    run.cancel.cancel();
                    self.cancel_job(port, run.job);
                    self.narration.failure = Some(Failed::Provider(Cause::Cancelled));
                }
                Ok(())
            }
            Action::Place { item } => self.place_narration(port, item),
            Action::Discard => {
                self.narration.discard();
                Ok(())
            }
            Action::SetDucking(ducking) => self.set_narration_ducking(port, ducking),
        }
    }

    /// Send `dub.cancel` for a job; a job that already ended refuses and that is fine.
    fn cancel_job(&mut self, port: &mut Port<'_>, job: JobId) {
        let budget = port.read.budget.clone();
        let _ = port.with_session(|session| {
            Ok(self
                .narration
                .control(session, &budget, Command::DubCancel { job }, None))
        });
    }

    /// Submit `dub.generate` for the text item. Language, voice and key refusals come back
    /// typed before any job exists; consent and the single provider attempt run inside it.
    fn start_narration(
        &mut self,
        port: &mut Port<'_>,
        item: ItemId,
        speed: Option<u16>,
    ) -> Result<()> {
        if self.narration.run.is_some() {
            return Ok(());
        }
        if port.view().session.transaction_active() {
            return Err(Error::Invalid("finish the open edit before generating"));
        }
        let facts = text_facts(port.view().session.project(), item)
            .ok_or(Error::Invalid("narration item"))?;
        let cue = facts.2;
        let text = self
            .narration
            .text
            .as_ref()
            .filter(|(bound, _)| *bound == item)
            .map_or_else(|| facts.0.clone(), |(_, text)| text.clone());
        // A fresh generation after a "shorten" advice is the rephrase the fit rules count.
        if matches!(
            self.narration.ready.as_ref().map(|ready| ready.fit),
            Some(Ok(Fit::Rephrase { .. }))
        ) {
            self.narration.rephrased = true;
        }
        self.narration.ready = None;
        self.narration.placed = None;
        self.narration.failure = None;
        let text = text.trim().to_owned();
        if text.is_empty() {
            self.narration.failure = Some(Failed::EmptyText);
            return Ok(());
        }
        let form = self.narration.form;
        let (provider, model, voice, language) = {
            let option = self
                .narration
                .option()
                .ok_or(Error::Invalid("speech provider"))?;
            (
                option.id.to_owned(),
                option
                    .models
                    .get(form.model)
                    .ok_or(Error::Invalid("speech model"))?
                    .clone(),
                option
                    .voices
                    .get(form.voice)
                    .ok_or(Error::Invalid("speech voice"))?
                    .0
                    .to_owned(),
                LanguageTag::parse(
                    LANGUAGES
                        .get(form.language)
                        .ok_or(Error::Invalid("speech language"))?,
                )?,
            )
        };
        let opener = self.narration.opener.clone().unwrap_or_else(|| {
            Arc::new(CredentialSpeech::new(Credentials::from_env())) as Arc<dyn SpeechOpener>
        });
        let cancel = Cancellation::new();
        let events = Arc::new(TaskEvents::new(cancel.clone()));
        let consent: ai_access::Consent = events.clone();
        let access = ai_access::narration_access(consent, opener);
        let budget = port.read.budget.clone();
        let command = Command::DubGenerate {
            provider,
            model,
            voice,
            language,
            text: NfcText::from(text.as_str()),
            cue_duration: cue,
            provider_speed_thousandths: speed,
        };
        let accepted = port.with_session(|session| {
            Ok(self
                .narration
                .control(session, &budget, command, Some(&access)))
        })?;
        match accepted {
            Ok(ControlResult::JobAccepted { job, .. }) => {
                self.narration.run = Some(Run {
                    events,
                    cancel,
                    job,
                    item,
                    cue,
                });
            }
            Ok(_) => return Err(Error::Invalid("narration job")),
            Err(error) => self.narration.failure = Some(failed_of_control(&error)),
        }
        Ok(())
    }

    /// Advance the `dub.generate` job once per frame and decide the finished take's fit.
    pub(in crate::editor) fn poll_narration(&mut self, port: &mut Port<'_>) {
        if let Some(job) = self.narration.orphan.take() {
            self.cancel_job(port, job);
        }
        let Some((job, item, cue)) = self
            .narration
            .run
            .as_ref()
            .map(|run| (run.job, run.item, run.cue))
        else {
            return;
        };
        let Some(runner) = self.narration.runner.as_mut() else {
            self.narration.run = None;
            return;
        };
        if runner.tick().is_err() {
            self.narration.run = None;
            self.narration.failure = Some(Failed::Provider(Cause::Other));
            return;
        }
        runner.drain_events();
        let Some(snapshot) = runner.snapshot(job) else {
            self.narration.run = None;
            self.narration.failure = Some(Failed::Provider(Cause::Other));
            return;
        };
        let outcome = match snapshot.state {
            JobState::Completed => match runner.output_bytes(job).map(decode_take) {
                Some(Ok(take)) => Ok(take),
                _ => Err(Failed::Provider(Cause::Other)),
            },
            JobState::Failed => Err(snapshot
                .error
                .as_ref()
                .map_or(Failed::Provider(Cause::Other), |error| {
                    failed_of_code(error.code())
                })),
            JobState::Cancelled => Err(Failed::Provider(Cause::Cancelled)),
            _ => return,
        };
        self.narration.run = None;
        let take = match outcome {
            Ok(take) => take,
            Err(failed) => {
                self.narration.failure = Some(failed);
                return;
            }
        };
        let Some((_, start, duration)) = text_facts(port.view().session.project(), item) else {
            self.narration.failure = Some(Failed::Stale);
            return;
        };
        if duration != cue {
            self.narration.failure = Some(Failed::Stale);
            return;
        }
        let Ok(natural) = take.natural() else {
            self.narration.failure = Some(Failed::Provider(Cause::Rejected));
            return;
        };
        let speed_offered = self
            .narration
            .options
            .iter()
            .any(|option| option.id == take.narration.provider.as_str() && option.speed)
            && take.narration.provider_speed_thousandths.is_none();
        let cue_end = start.checked_add(cue).unwrap_or(start);
        let fitted = fit(FitInput {
            natural,
            cue,
            silent_after: silent_after(port.view().session.project(), cue_end),
            characters: take.narration.characters,
            provider_speed: speed_offered,
            rephrased: self.narration.rephrased,
        });
        self.narration.ready = Some(Ready {
            take,
            item,
            cue,
            fit: fitted,
        });
    }

    /// Place the take through `dub.place`: asset, item, stretch and ducking in one undo step.
    fn place_narration(&mut self, port: &mut Port<'_>, item: ItemId) -> Result<()> {
        let Some(ready) = self.narration.ready.as_ref() else {
            return Ok(());
        };
        let thousandths = match ready.fit {
            Ok(Fit::Exact | Fit::Gap) => 1000,
            Ok(Fit::Stretch { thousandths } | Fit::Overflow { thousandths, .. }) => thousandths,
            // A take that needs regeneration or cannot fit has no placement.
            _ => return Err(Error::Invalid("narration fit")),
        };
        let current = text_facts(port.view().session.project(), item);
        let Some((_, start, _)) = current.filter(|facts| facts.2 == ready.cue) else {
            self.narration.failure = Some(Failed::Stale);
            return Ok(());
        };
        if ready.item != item {
            self.narration.failure = Some(Failed::Stale);
            return Ok(());
        }
        let directory = self
            .narration
            .directory
            .clone()
            .unwrap_or_else(|| project::media_dir(port.session.saved_path()));
        let Ok(path) = write_wav(&directory, &ready.take) else {
            self.narration.failure = Some(Failed::Storage);
            return Ok(());
        };
        let next = port
            .view()
            .session
            .project()
            .data()
            .assets
            .iter()
            .map(|asset| asset.id().0)
            .max()
            .map_or(1, |id| id.saturating_add(1));
        let asset = SourceAsset::import(AssetId(next), path)
            .map_err(|_| Error::Invalid("narration asset"))?;
        let speed = time::Speed::from_thousandths(u32::from(thousandths))?;
        let narration = ready.take.narration.clone();
        let outcome = port.apply_result(Command::DubPlace {
            placement: engine::Placement::Auto,
            start,
            asset,
            narration,
            speed,
        });
        match outcome {
            Ok(outcome) => {
                self.narration.placed = outcome.created.iter().find_map(|id| match id {
                    CreatedId::Item(placed) => Some(*placed),
                    _ => None,
                });
                self.narration.ready = None;
                self.narration.rephrased = false;
                self.narration.failure = None;
                self.narration.ducking_draft = None;
            }
            Err(_) => self.narration.failure = Some(Failed::Placement),
        }
        Ok(())
    }

    /// Ducking edits the pending take before placement, and the placed item after it.
    fn set_narration_ducking(
        &mut self,
        port: &mut Port<'_>,
        ducking: NarrationDucking,
    ) -> Result<()> {
        if let Some(ready) = self.narration.ready.as_mut() {
            ready.take.narration.ducking = ducking;
            return Ok(());
        }
        if let Some(item) = self.narration.placed {
            port.apply_result(Command::DubSetDucking { item, ducking })?;
        }
        Ok(())
    }
}
