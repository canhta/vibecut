//! Keyword emphasis of the caption cue under the playhead: manual word picks, the lexicon
//! choice, preview, clear and the consented AI suggestion. The properties area draws the section
//! from `keyword_ui` and sends `keyword_action`s back through the frame. Each gesture is one
//! engine command, so it is one undo step; AI suggestions apply as one batch.
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
use project::{CaptionParams, Item, ItemId, Project};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
use time::Tick;

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

/// What the section shows for one frame, read from the caption item at the playhead.
struct View {
    item: ItemId,
    /// The caption cue under the playhead, if one is.
    cue: Option<captions::Cue>,
    /// The cue under the playhead carries verified word timing.
    timed: bool,
    /// Timed words of the cue: word id and text.
    words: Vec<(u32, String)>,
    /// Most words the cue may emphasise.
    limit: usize,
    /// The cue's emphasis that still validates against its current source text.
    shown: Vec<u32>,
    /// A stored selection no longer validates, for example after its source text changed.
    stale: bool,
    highlight: bool,
}

/// The timeline start and end of a caption cue: the mapping the cue list seeks by.
fn cue_span(item: &Item, cue: &captions::Cue) -> Option<(Tick, Tick)> {
    let at = |source: Tick| {
        source
            .checked_sub(item.source_range.start())
            .and_then(|offset| item.start.checked_add(offset))
    };
    Some((at(cue.range.start())?, at(cue.range.end())?))
}

/// The cue whose timeline span holds the playhead. Its end is open, so a playhead between two
/// cues, or past the last one, is under no cue.
fn cue_at(item: &Item, params: &CaptionParams, playhead: Tick) -> Option<captions::Cue> {
    params.cues.cues.iter().find_map(|cue| {
        let (start, end) = cue_span(item, cue)?;
        (start <= playhead && playhead < end).then(|| cue.clone())
    })
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
    /// What the section shows for the cue under the playhead.
    fn emphasis_view(&self, session: &Session, playhead: Tick) -> Option<View> {
        let item = self.item?;
        let (caption, params) = self.caption(session)?;
        let cue = cue_at(caption, params, playhead);
        let (words, limit, shown, stale) = match &cue {
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
            timed: cue.as_ref().is_some_and(|cue| !cue.words().is_empty()),
            cue,
            words,
            limit,
            shown,
            stale,
            highlight: params.word_highlight,
        })
    }

    /// The keyword section's read for the caption item the properties area shows, with the
    /// cue under `playhead`. `None` unless that item is the Captions panel's current item.
    pub(crate) fn keyword_ui(
        &self,
        session: &Session,
        item: ItemId,
        playhead: Tick,
    ) -> Option<KeywordUi> {
        if self.item != Some(item) {
            return None;
        }
        let view = self.emphasis_view(session, playhead)?;
        let owned = self.emphasis.item == Some(item);
        let run = match &self.suggest {
            None => KeywordRun::Idle,
            Some(_) if self.keywords_awaiting_approval() => KeywordRun::AwaitingApproval,
            Some(_) => KeywordRun::Working,
        };
        Some(KeywordUi {
            item,
            cue: view.cue.as_ref().map(|cue| cue.id.clone()),
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

    /// Apply one keyword intention from the properties area to the cue under `playhead`. Edits
    /// wait while other subtitle work runs; cancelling stops only the suggestion run.
    pub(crate) fn keyword_action(
        &mut self,
        session: &mut Session,
        options: &Options,
        action: KeywordAction,
        playhead: Tick,
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
        let Some(view) = self.emphasis_view(session, playhead) else {
            return;
        };
        match action {
            KeywordAction::Choose => {
                if let Some(cue) = &view.cue {
                    self.commit_emphasis(
                        session,
                        Command::SelectCaptionEmphasis {
                            item: view.item,
                            cue_ids: vec![cue.id.clone()],
                            mode: EmphasisMode::Lexicon,
                        },
                    );
                }
            }
            KeywordAction::Clear => {
                if let Some(cue) = &view.cue {
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
                if let Some(cue) = &view.cue {
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
                if let Some(cue) = &view.cue {
                    self.preview_emphasis(session, cue);
                }
            }
            KeywordAction::Suggest => {
                if let Some(cue) = &view.cue {
                    self.suggest_emphasis(session, options, view.item, vec![cue.id.clone()]);
                }
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

    /// Move the playhead to the start of the cue, where the player shows its keywords. The
    /// Captions panel's cue selection is left alone.
    fn preview_emphasis(&mut self, session: &Session, cue: &captions::Cue) {
        self.emphasis.problem = None;
        let start = self
            .caption(session)
            .and_then(|(item, _)| cue_span(item, cue))
            .map(|(start, _)| start);
        match start {
            Some(start) => self.requested_seek = Some(start),
            None => self.emphasis.problem = Some(Text::SubEditFailed),
        }
    }

    /// Start one consented suggestion for the cue under the playhead, with the model the
    /// preferences snapshot resolved for the provider's keyword selection.
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
        wake::spawn(move || {
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
