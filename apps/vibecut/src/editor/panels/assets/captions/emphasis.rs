//! Keyword emphasis section of the Captions tab: manual word picks, the lexicon choice,
//! preview, clear and the consented AI suggestion. Each gesture is one engine command, so it
//! is one undo step; AI suggestions apply as one batch.
use super::Subtitles;
use super::view::Controls;
use crate::{
    egui,
    i18n::Text,
    kit::Icon,
    preferences::SubtitleOptions as Options,
    services::{
        ai_access::{Consent, TextOpener},
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
    let access = (input.opener)(&input.provider, &input.model, input.consent)
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
        let (shown, stale) = match &target {
            Some(cue) => {
                let stored = params.emphasis.get(&cue.id).cloned().unwrap_or_default();
                let shown = captions::resolve_selection(cue, &stored).to_vec();
                let stale = !stored.is_empty() && shown.is_empty();
                (shown, stale)
            }
            None => (Vec::new(), false),
        };
        Some(View {
            item,
            selected,
            target,
            timed,
            shown,
            stale,
            highlight: params.word_highlight,
        })
    }

    /// The keyword section under the cue list.
    pub(super) fn emphasis_ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        c: &mut Controls,
        options: &Options,
    ) {
        if self.emphasis.item != self.item {
            self.emphasis = Panel {
                item: self.item,
                ..Panel::default()
            };
        }
        let Some(view) = self.emphasis_view(session) else {
            return;
        };
        let idle = !self.busy();
        let running = self.suggest.is_some();
        let awaiting = self.keywords_awaiting_approval();
        let mut choose = false;
        let mut clear = false;
        let mut preview = false;
        let mut ai = false;
        let mut apply = false;
        let mut cancel = false;
        let mut pick: Option<Vec<u32>> = None;
        let header = egui::CollapsingHeader::new(c.tr(Text::EmphasisTitle))
            .id_salt("subtitles.emphasis")
            .show(ui, |ui| {
                ui.add_enabled_ui(idle, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        choose = c.button(
                            ui,
                            "subtitles.emphasis.choose",
                            Text::EmphasisChoose,
                            Icon::TextT,
                            view.timed,
                        );
                        preview = c.button(
                            ui,
                            "subtitles.emphasis.preview",
                            Text::EmphasisPreview,
                            Icon::Play,
                            view.target.is_some(),
                        );
                        clear = c.button(
                            ui,
                            "subtitles.emphasis.clear",
                            Text::EmphasisClear,
                            Icon::X,
                            view.target.is_some() && (!view.shown.is_empty() || view.stale),
                        );
                        ai = c.button(
                            ui,
                            "subtitles.emphasis.ai",
                            Text::EmphasisAi,
                            Icon::Sparkle,
                            !view.selected.is_empty(),
                        );
                    });
                    if view.selected.len() > 1 {
                        ui.label(c.tr(Text::EmphasisOneCue));
                    }
                    if let Some(cue) = &view.target {
                        let words = cue.words();
                        if words.is_empty() {
                            ui.label(c.tr(Text::EmphasisNoTimings));
                        } else {
                            let limit = captions::emphasis_limit(words.len());
                            ui.horizontal_wrapped(|ui| {
                                for (index, word) in words.iter().enumerate() {
                                    let id = index as u32;
                                    let on = view.shown.contains(&id);
                                    let full = !on && view.shown.len() >= limit;
                                    let r = ui.add_enabled(
                                        !full,
                                        egui::Button::new(word.text.as_str()).selected(on),
                                    );
                                    c.remember(format!("subtitles.emphasis.word.{index}"), &r);
                                    if r.clicked() {
                                        let mut next = view.shown.clone();
                                        if on {
                                            next.retain(|value| *value != id);
                                        } else {
                                            next.push(id);
                                            next.sort_unstable();
                                        }
                                        pick = Some(next);
                                    }
                                }
                            });
                        }
                    }
                    if view.stale {
                        ui.label(c.tr(Text::EmphasisStale));
                    } else if view.target.is_some() && !view.highlight {
                        ui.label(c.tr(Text::EmphasisHighlightOff));
                    }
                });
                if running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let waiting = if awaiting {
                            Text::EmphasisAwaitingApproval
                        } else {
                            Text::EmphasisWorking
                        };
                        ui.label(c.tr(waiting));
                        cancel =
                            c.button(ui, "subtitles.emphasis.cancel", Text::Cancel, Icon::X, true);
                    });
                }
                if let Some(problem) = self.emphasis.problem {
                    ui.label(c.tr(problem));
                }
                if !self.emphasis.suggestions.is_empty() {
                    apply = c.button(
                        ui,
                        "subtitles.emphasis.apply",
                        Text::EmphasisApply,
                        Icon::TextT,
                        idle,
                    );
                }
            });
        c.remember("subtitles.emphasis", &header.header_response);
        if cancel {
            self.cancel();
        }
        if choose {
            self.commit_emphasis(
                session,
                Command::SelectCaptionEmphasis {
                    item: view.item,
                    cue_ids: view.selected.clone(),
                    mode: EmphasisMode::Lexicon,
                },
            );
        }
        if let (Some(cue), Some(word_ids)) = (&view.target, pick) {
            self.commit_emphasis(
                session,
                Command::SetCaptionEmphasis {
                    item: view.item,
                    cue_id: cue.id.clone(),
                    word_ids,
                },
            );
        }
        if clear && let Some(cue) = &view.target {
            self.commit_emphasis(
                session,
                Command::SetCaptionEmphasis {
                    item: view.item,
                    cue_id: cue.id.clone(),
                    word_ids: Vec::new(),
                },
            );
        }
        if preview && let Some(cue) = &view.target {
            self.preview_emphasis(session, &cue.id);
        }
        if ai {
            self.suggest_emphasis(session, options, view.item, view.selected.clone());
        }
        if apply {
            self.apply_suggestions(session);
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

    /// Start one consented suggestion for the selected cues. The model comes from the
    /// provider's default for keyword selection, so the run never reads a stale snapshot.
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
            model: ai::defaults::model(&options.text_provider, ai::Operation::SelectEmphasis)
                .into(),
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
