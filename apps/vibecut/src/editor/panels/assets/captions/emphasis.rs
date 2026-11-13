//! Keyword emphasis of the selected caption cue: manual word picks, the lexicon choice,
//! preview, clear and the consented AI suggestion. The properties area draws the section from
//! `keyword_ui` and sends `keyword_action`s back through the frame. Each gesture is one engine
//! command, so it is one undo step; AI suggestions apply as one batch.
use super::Subtitles;
use crate::{
    editor::context::{KeywordAction, KeywordRun, KeywordUi},
    i18n::Text,
    preferences::SubtitleOptions as Options,
    services::{
        ai_access::{Consent, TextOpener, TextTask},
        ai_task::TaskEvents,
    },
};
use captions::NfcText;
use engine::{
    AnalysisError, ByteBudget, Command, EditError, EmphasisMode, Session,
    emphasis::EmphasisRequest, transcript::Cancellation,
};
use project::{ItemId, Project};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

/// The section's state. It resets when the caption item changes.
#[derive(Default)]
pub(super) struct Panel {
    item: Option<ItemId>,
    problem: Option<Text>,
    /// Suggested emphasis per cue, waiting for one apply.
    suggestions: Vec<(NfcText, Vec<u32>)>,
}

/// One suggestion run with the lent text provider. It runs on its own thread because
/// `EmphasisRequest::submit` requests the human consent synchronously; the sheet is the
/// captions panel's `TaskEvents`, shown like every other upload.
pub(super) struct Suggest {
    pub(super) events: Arc<TaskEvents>,
    pub(super) cancel: Cancellation,
    receive: mpsc::Receiver<Result<Found, Outcome>>,
}

/// Cue id and the positions the provider chose inside it.
type Found = Vec<(NfcText, Vec<u32>)>;

/// Why a suggestion run produced nothing.
#[derive(Clone, Copy)]
enum Outcome {
    Declined,
    Cancelled,
    Failed,
}

/// Everything the suggestion thread owns.
struct Input {
    project: Arc<Project>,
    item: ItemId,
    cues: Option<Vec<NfcText>>,
    budget: ByteBudget,
    opener: TextOpener,
    provider: String,
    model: String,
    consent: Consent,
    cancel: Cancellation,
}

/// What the section shows for one frame, read from the caption item.
struct View {
    item: ItemId,
    selected: Vec<NfcText>,
    /// The single selected cue, when exactly one is selected.
    target: Option<captions::Cue>,
    /// Every selected cue carries verified word timing.
    timed: bool,
    /// Timed words of the target cue: word id and text.
    words: Vec<(u32, String)>,
    /// Most words the target cue may emphasise.
    limit: usize,
    /// The target's emphasis that still validates against its current source text.
    shown: Vec<u32>,
    /// A stored selection no longer validates, for example after its source text changed.
    stale: bool,
    highlight: bool,
}

fn outcome(error: AnalysisError) -> Outcome {
    match error {
        AnalysisError::Providers(engine::providers::Error::ConsentRequired) => Outcome::Declined,
        AnalysisError::Runtime(engine::RuntimeError::Cancelled) => Outcome::Cancelled,
        _ => Outcome::Failed,
    }
}

/// Runs one suggestion job to completion on the calling thread (the worker, never the window).
fn suggest(input: Input) -> Result<Found, Outcome> {
    let request =
        EmphasisRequest::new(&input.project, input.item, input.cues.as_deref()).map_err(outcome)?;
    let access = (input.opener)(
        TextTask::Emphasis,
        &input.provider,
        &input.model,
        input.consent,
    )
    .map_err(|error| outcome(AnalysisError::from(error)))?;
    let mut runner = super::job_runner(&input.budget).map_err(|_| Outcome::Failed)?;
    let job = request.submit(&mut runner, &access).map_err(outcome)?;
    loop {
        if input.cancel.cancelled() {
            let _ = runner.cancel_job(job.id);
        }
        runner.tick().map_err(|_| Outcome::Failed)?;
        match job.finish(&mut runner) {
            Ok(Some(found)) => {
                return Ok(found
                    .into_iter()
                    .map(|suggestion| (suggestion.cue, suggestion.word_ids))
                    .collect());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => return Err(outcome(error)),
        }
    }
}

/// The creator-facing reason for a refused emphasis command.
fn refusal(error: &EditError) -> Text {
    match error {
        EditError::Captions(captions::Error::NoWordTiming) => Text::EmphasisNoTimings,
        EditError::Locked(_) => Text::SubTranscriptLocked,
        _ => Text::SubEditFailed,
    }
}

impl Subtitles {
    /// What the section shows for the selected cues of the caption item.
    fn emphasis_view(&self, session: &Session) -> Option<View> {
        let item = self.item?;
        let (_, params) = self.caption(session)?;
        let selected: Vec<NfcText> = self.selected.iter().cloned().collect();
        let mut target = None;
        let mut timed = !selected.is_empty();
        for cue in &params.cues.cues {
            if self.selected.contains(&cue.id) {
                timed &= !cue.words().is_empty();
                if selected.len() == 1 {
                    target = Some(cue.clone());
                }
            }
        }
        let (words, limit, shown, stale) = match &target {
            Some(cue) => {
                let timed_words = cue.words();
                let words = timed_words
                    .iter()
                    .enumerate()
                    .map(|(index, word)| (index as u32, word.text.as_str().to_owned()))
                    .collect();
                let stored = params.emphasis.get(&cue.id).cloned().unwrap_or_default();
                let shown = captions::resolve_selection(cue, &stored).to_vec();
                let stale = !stored.is_empty() && shown.is_empty();
                (
                    words,
                    captions::emphasis_limit(timed_words.len()),
                    shown,
                    stale,
                )
            }
            None => (Vec::new(), 0, Vec::new(), false),
        };
        Some(View {
            item,
            selected,
            target,
            timed,
            words,
            limit,
            shown,
            stale,
            highlight: params.word_highlight,
        })
    }

    /// The keyword section's read for the caption item the properties area shows. `None`
    /// unless that item is the Captions panel's current item.
    pub(crate) fn keyword_ui(&self, session: &Session, item: ItemId) -> Option<KeywordUi> {
        if self.item != Some(item) {
            return None;
        }
        let view = self.emphasis_view(session)?;
        let owned = self.emphasis.item == Some(item);
        let run = match &self.suggest {
            None => KeywordRun::Idle,
            Some(_) if self.keywords_awaiting_approval() => KeywordRun::AwaitingApproval,
            Some(_) => KeywordRun::Working,
        };
        Some(KeywordUi {
            item,
            cue: view.target.as_ref().map(|cue| cue.id.clone()),
            selected: view.selected.len(),
            timed: view.timed,
            words: view.words,
            shown: view.shown,
            limit: view.limit,
            stale: view.stale,
            highlight: view.highlight,
            suggested: if owned {
                self.emphasis.suggestions.len()
            } else {
                0
            },
            run,
            problem: if owned { self.emphasis.problem } else { None },
            busy: self.busy(),
        })
    }

    /// Apply one keyword intention from the properties area. Edits wait while other subtitle
    /// work runs; cancelling stops only the suggestion run.
    pub(crate) fn keyword_action(
        &mut self,
        session: &mut Session,
        options: &Options,
        action: KeywordAction,
    ) {
        if self.emphasis.item != self.item {
            self.emphasis = Panel {
                item: self.item,
                ..Panel::default()
            };
        }
        if action == KeywordAction::Cancel {
            if let Some(run) = &self.suggest {
                run.cancel.cancel();
            }
            return;
        }
        if self.busy() {
            return;
        }
        let Some(view) = self.emphasis_view(session) else {
            return;
        };
        match action {
            KeywordAction::Choose => self.commit_emphasis(
                session,
                Command::SelectCaptionEmphasis {
                    item: view.item,
                    cue_ids: view.selected.clone(),
                    mode: EmphasisMode::Lexicon,
                },
            ),
            KeywordAction::Clear => {
                if let Some(cue) = &view.target {
                    self.commit_emphasis(
                        session,
                        Command::SetCaptionEmphasis {
                            item: view.item,
                            cue_id: cue.id.clone(),
                            word_ids: Vec::new(),
                        },
                    );
                }
            }
            KeywordAction::Pick(word_ids) => {
                if let Some(cue) = &view.target {
                    self.commit_emphasis(
                        session,
                        Command::SetCaptionEmphasis {
                            item: view.item,
                            cue_id: cue.id.clone(),
                            word_ids,
                        },
                    );
                }
            }
            KeywordAction::Preview => {
                if let Some(cue) = &view.target {
                    self.preview_emphasis(session, &cue.id);
                }
            }
            KeywordAction::Suggest => {
                self.suggest_emphasis(session, options, view.item, view.selected.clone());
            }
            KeywordAction::Apply => self.apply_suggestions(session),
            KeywordAction::Cancel => {}
        }
    }

    fn keywords_awaiting_approval(&self) -> bool {
        self.suggest.as_ref().is_some_and(|run| {
            run.events
                .state
                .lock()
                .is_ok_and(|state| state.upload.is_some())
        })
    }

    /// Flush open drafts, then run one emphasis command. A refusal shows its reason and
    /// changes nothing; success is one undo step.
    fn commit_emphasis(&mut self, session: &mut Session, command: Command) {
        self.emphasis.problem = None;
        if self.flush_drafts(session).is_err() {
            self.emphasis.problem = Some(Text::SubEditFailed);
            return;
        }
        if let Err(error) = session.apply(command) {
            self.emphasis.problem = Some(refusal(&error));
        }
    }

    /// Select the cue and move the playhead to it, where the player shows its keywords.
    fn preview_emphasis(&mut self, session: &mut Session, cue: &NfcText) {
        self.emphasis.problem = None;
        if self.flush_drafts(session).is_err() {
            self.emphasis.problem = Some(Text::SubEditFailed);
            return;
        }
        let index = self.caption(session).and_then(|(_, params)| {
            params
                .cues
                .cues
                .iter()
                .position(|candidate| candidate.id == *cue)
        });
        let Some(index) = index else {
            return;
        };
        match self.select_cue(session, index, false, false) {
            Ok(at) => self.requested_seek = Some(at),
            Err(_) => self.emphasis.problem = Some(Text::SubEditFailed),
        }
    }

    /// Start one consented suggestion for the selected cues, with the model the preferences
    /// snapshot resolved for the provider's keyword selection.
    fn suggest_emphasis(
        &mut self,
        session: &mut Session,
        options: &Options,
        item: ItemId,
        cues: Vec<NfcText>,
    ) {
        if self.busy() {
            return;
        }
        self.emphasis.problem = None;
        self.emphasis.suggestions.clear();
        if self.flush_drafts(session).is_err() {
            self.emphasis.problem = Some(Text::SubEditFailed);
            return;
        }
        let cancel = Cancellation::new();
        let events = Arc::new(TaskEvents::new(cancel.clone()));
        let consent: Consent = events.clone();
        let input = Input {
            project: session.project().clone(),
            item,
            cues: Some(cues),
            budget: self.budget.clone(),
            opener: self.text.clone(),
            provider: options.text_provider.clone(),
            model: options.emphasis_model.clone(),
            consent,
            cancel: cancel.clone(),
        };
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = send.send(suggest(input));
        });
        self.suggest = Some(Suggest {
            events,
            cancel,
            receive,
        });
    }

    /// Apply a finished suggestion run. Called on every tick, so the window never waits for it.
    pub(super) fn poll_suggest(&mut self) {
        let Some(run) = &self.suggest else {
            return;
        };
        let result = match run.receive.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(Outcome::Failed),
        };
        self.suggest = None;
        match result {
            Ok(found) => {
                self.emphasis.problem = None;
                self.emphasis.suggestions = found;
            }
            Err(Outcome::Cancelled) => {}
            Err(Outcome::Declined) => self.emphasis.problem = Some(Text::EmphasisDeclined),
            Err(Outcome::Failed) => self.emphasis.problem = Some(Text::EmphasisAiFailed),
        }
    }

    /// Apply every suggested cue in one batch, so the whole suggestion is one undo step.
    fn apply_suggestions(&mut self, session: &mut Session) {
        let Some(item) = self.item else {
            return;
        };
        let commands: Vec<Command> = std::mem::take(&mut self.emphasis.suggestions)
            .into_iter()
            .map(|(cue_id, word_ids)| Command::SetCaptionEmphasis {
                item,
                cue_id,
                word_ids,
            })
            .collect();
        if commands.is_empty() {
            return;
        }
        self.commit_emphasis(session, Command::Batch { commands });
    }
}
