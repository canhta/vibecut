//! Filler cut list section of the Captions tab: candidate state, reads and removal.
use super::Subtitles;
use super::view::{Controls, time_text};
use crate::{
    egui,
    i18n::{self, Locale, Text},
    kit::Icon,
    preferences::SubtitleOptions as Options,
    services::{
        ai_access::{self, Consent, TextOpener, TextTask},
        ai_task::TaskEvents,
    },
};
use captions::{CueWordTiming, FillerKind, FillerLexicon, LanguageTag, NfcText};
use engine::{
    AnalysisError, ByteBudget, Session,
    control::{ControlResult, FillerCandidate},
    fillers::FillerRequest,
    transcript::Cancellation,
};
use project::{ItemId, ItemParams, Project};
use std::{
    collections::BTreeSet,
    sync::{Arc, mpsc},
    time::Duration,
};

/// The cut list's state. Candidates belong to one caption item; a different item clears it.
#[derive(Default)]
pub(super) struct Cuts {
    item: Option<ItemId>,
    scanned: bool,
    candidates: Vec<FillerCandidate>,
    /// Candidate ids the next removal passes to `timeline.remove_fillers`.
    selected: BTreeSet<NfcText>,
    /// Ids waiting for the edited-cue confirmation row.
    confirm: Option<Vec<NfcText>>,
    problem: Option<Text>,
    removed: Option<usize>,
}

/// One classification with the lent text provider. It runs on its own thread because
/// `FillerRequest::submit` requests the human consent synchronously; the sheet itself is the
/// captions panel's `TaskEvents`, shown by the same dialog as every other upload.
pub(super) struct Classify {
    pub(super) events: Arc<TaskEvents>,
    pub(super) cancel: Cancellation,
    receive: mpsc::Receiver<Result<Vec<FillerCandidate>, Outcome>>,
}

/// Why a classification produced no candidates.
#[derive(Clone, Copy)]
enum Outcome {
    Declined,
    Cancelled,
    Failed,
}

/// Everything the classification thread owns.
struct Input {
    project: Arc<Project>,
    language: LanguageTag,
    budget: ByteBudget,
    opener: TextOpener,
    provider: String,
    model: String,
    consent: Consent,
    cancel: Cancellation,
}

fn outcome(error: AnalysisError) -> Outcome {
    match error {
        AnalysisError::Providers(engine::providers::Error::ConsentRequired) => Outcome::Declined,
        AnalysisError::Runtime(engine::RuntimeError::Cancelled) => Outcome::Cancelled,
        _ => Outcome::Failed,
    }
}

/// Runs one filler job to completion on the calling thread (the worker, never the window).
fn classify(input: Input) -> Result<Vec<FillerCandidate>, Outcome> {
    let request = FillerRequest::new(&input.project, input.language, false).map_err(outcome)?;
    let access = (input.opener)(
        TextTask::Fillers,
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
            Ok(Some(candidates)) => return Ok(candidates),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => return Err(outcome(error)),
        }
    }
}

impl Subtitles {
    /// The cut list section under the cue list. Its state resets when the caption item changes.
    pub(super) fn fillers_ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        c: &mut Controls,
        locale: Locale,
        options: &Options,
    ) {
        if self.fillers.item != self.item {
            self.fillers = Cuts {
                item: self.item,
                ..Cuts::default()
            };
        }
        let busy = self.busy();
        let idle = !busy;
        let rate = session.project().data().canvas.fps;
        let mut find = false;
        let mut find_ai = false;
        let mut apply = false;
        let mut cancel = false;
        let mut answer = None;
        let header = egui::CollapsingHeader::new(c.tr(Text::FillersTitle))
            .id_salt("subtitles.fillers")
            .show(ui, |ui| {
                ui.add_enabled_ui(idle, |ui| {
                    find = c.button(
                        ui,
                        "subtitles.fillers.find",
                        Text::FillersFind,
                        Icon::MagnifyingGlass,
                        true,
                    );
                    find_ai = c.button(
                        ui,
                        "subtitles.fillers.ai",
                        Text::FillersFindAi,
                        Icon::Sparkle,
                        true,
                    );
                });
                if self.classify.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let waiting = if self.awaiting_approval() {
                            Text::FillersAwaitingApproval
                        } else {
                            Text::FillersWorking
                        };
                        ui.label(c.tr(waiting));
                        cancel =
                            c.button(ui, "subtitles.fillers.cancel", Text::Cancel, Icon::X, true);
                    });
                }
                ui.add_enabled_ui(idle, |ui| {
                    if self.fillers.scanned {
                        if let Some(problem) = self.fillers.problem {
                            ui.label(c.tr(problem));
                        } else if self.fillers.candidates.is_empty() {
                            ui.label(c.tr(Text::FillersNone));
                        } else {
                            for candidate in self.fillers.candidates.clone() {
                                let mut on = self.fillers.selected.contains(&candidate.id);
                                let kind = match candidate.kind {
                                    FillerKind::FilledPause => Text::FillersKindPause,
                                    FillerKind::DiscourseMarker => Text::FillersKindMarker,
                                };
                                let label = format!(
                                    "{} · {}",
                                    time_text(candidate.range.start, rate, locale),
                                    c.tr(kind)
                                );
                                let r = ui.checkbox(&mut on, label);
                                c.remember(
                                    format!("subtitles.fillers.toggle.{}", candidate.id.as_str()),
                                    &r,
                                );
                                if r.changed() {
                                    if on {
                                        self.fillers.selected.insert(candidate.id.clone());
                                    } else {
                                        self.fillers.selected.remove(&candidate.id);
                                    }
                                }
                            }
                            apply = c.button(
                                ui,
                                "subtitles.fillers.apply",
                                Text::FillersApply,
                                Icon::Scissors,
                                !self.fillers.selected.is_empty(),
                            );
                        }
                    }
                    if self.fillers.confirm.is_some() {
                        ui.label(c.tr(Text::FillersEdited));
                        if c.button(
                            ui,
                            "subtitles.fillers.confirm",
                            Text::FillersConfirm,
                            Icon::Scissors,
                            true,
                        ) {
                            answer = Some(true);
                        }
                        if c.button(
                            ui,
                            "subtitles.fillers.confirm.cancel",
                            Text::Cancel,
                            Icon::X,
                            true,
                        ) {
                            answer = Some(false);
                        }
                    }
                    if let Some(count) = self.fillers.removed {
                        ui.label(i18n::format(
                            locale,
                            Text::FillersRemoved,
                            &[("count", &i18n::number(locale, count as u64))],
                        ));
                    }
                });
            });
        c.remember("subtitles.fillers", &header.header_response);
        if cancel {
            self.cancel();
        }
        match answer {
            Some(true) => {
                if let Some(ids) = self.fillers.confirm.take() {
                    self.remove_fillers(session, ids, true);
                }
            }
            Some(false) => self.fillers.confirm = None,
            None => {}
        }
        if apply {
            let ids: Vec<NfcText> = self
                .fillers
                .candidates
                .iter()
                .filter(|candidate| self.fillers.selected.contains(&candidate.id))
                .map(|candidate| candidate.id.clone())
                .collect();
            let edited = self.fillers.candidates.iter().any(|candidate| {
                candidate.edited_cue && self.fillers.selected.contains(&candidate.id)
            });
            if edited {
                self.fillers.confirm = Some(ids);
            } else {
                self.remove_fillers(session, ids, false);
            }
        }
        if find {
            self.scan_fillers(session);
        }
        if find_ai {
            self.find_with_ai(session, options);
        }
    }

    /// The language the caption item's first cue is in, or why no candidates can be read.
    fn filler_language(&self, session: &Session) -> Result<LanguageTag, Text> {
        // The read is scene-wide, and a word cut splits a caption item, so the cues of every
        // caption item decide the language, not only the selected piece (which may hold none).
        let cues: Vec<_> = session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .filter_map(|item| match &item.content {
                ItemParams::Caption(params) => Some(params),
                _ => None,
            })
            .flat_map(|params| params.cues.cues.iter())
            .collect();
        let language = cues.first().map(|cue| cue.source_lang.clone());
        let timed = cues
            .iter()
            .any(|cue| matches!(cue.word_timing, CueWordTiming::Words { .. }));
        let language = language.filter(|_| timed).ok_or(Text::FillersNoTimings)?;
        FillerLexicon::builtin(&language).map_err(|_| Text::FillersUnsupported)?;
        Ok(language)
    }

    /// Clear the list for a new read: every candidate starts checked once loaded.
    fn reset_list(&mut self) {
        self.fillers.scanned = true;
        self.fillers.candidates.clear();
        self.fillers.selected.clear();
        self.fillers.confirm = None;
        self.fillers.removed = None;
        self.fillers.problem = None;
    }

    fn load(&mut self, candidates: Vec<FillerCandidate>) {
        self.fillers.selected = candidates
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect();
        self.fillers.candidates = candidates;
        self.fillers.problem = None;
    }

    /// Read the lexicon candidates of the caption item (`timeline.filler_candidates`).
    fn scan_fillers(&mut self, session: &mut Session) {
        self.reset_list();
        let language = match self.filler_language(session) {
            Ok(language) => language,
            Err(problem) => {
                self.fillers.problem = Some(problem);
                return;
            }
        };
        let result = ai_access::run(
            session,
            &mut self.runner,
            None,
            None,
            "timeline.filler_candidates",
            serde_json::json!({"language": language.as_str(), "include_markers": false}),
        );
        match result {
            Ok(ControlResult::Fillers { candidates }) => self.load(candidates),
            _ => self.fillers.problem = Some(Text::FillersFailed),
        }
    }

    /// Start the classification with the configured text provider. The upload sheet is the
    /// captions panel's own, so the run is refused while any other subtitle job or sheet is open.
    fn find_with_ai(&mut self, session: &mut Session, options: &Options) {
        if self.busy() {
            return;
        }
        self.reset_list();
        let language = match self.filler_language(session) {
            Ok(language) => language,
            Err(problem) => {
                self.fillers.problem = Some(problem);
                return;
            }
        };
        if self.flush_drafts(session).is_err() {
            self.fillers.problem = Some(Text::FillersFailed);
            return;
        }
        let cancel = Cancellation::new();
        let events = Arc::new(TaskEvents::new(cancel.clone()));
        let consent: Consent = events.clone();
        let input = Input {
            project: session.project().clone(),
            language,
            budget: self.budget.clone(),
            opener: self.text.clone(),
            provider: options.text_provider.clone(),
            model: options.fillers_model.clone(),
            consent,
            cancel: cancel.clone(),
        };
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = send.send(classify(input));
        });
        self.classify = Some(Classify {
            events,
            cancel,
            receive,
        });
    }

    fn awaiting_approval(&self) -> bool {
        self.classify.as_ref().is_some_and(|run| {
            run.events
                .state
                .lock()
                .is_ok_and(|state| state.upload.is_some())
        })
    }

    /// Apply a finished classification. Called on every tick, so the window never waits for it.
    pub(super) fn poll_classify(&mut self) {
        let Some(run) = &self.classify else {
            return;
        };
        let result = match run.receive.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(Outcome::Failed),
        };
        self.classify = None;
        match result {
            Ok(candidates) => {
                self.reset_list();
                self.load(candidates);
            }
            Err(Outcome::Cancelled) => {}
            Err(Outcome::Declined) => {
                self.reset_list();
                self.fillers.problem = Some(Text::FillersDeclined);
            }
            Err(Outcome::Failed) => {
                self.reset_list();
                self.fillers.problem = Some(Text::FillersAiFailed);
            }
        }
    }

    /// Remove the passed candidates in one registry command, one undo step. The list then
    /// clears: the words came from one reading, and a fresh read is the creator's next step.
    fn remove_fillers(&mut self, session: &mut Session, ids: Vec<NfcText>, confirm_edited: bool) {
        let count = ids.len();
        if self.flush_drafts(session).is_err() {
            self.fillers.problem = Some(Text::FillersFailed);
            return;
        }
        let result = ai_access::run(
            session,
            &mut self.runner,
            None,
            None,
            "timeline.remove_fillers",
            serde_json::json!({"ids": ids, "confirm_edited": confirm_edited}),
        );
        self.reset_list();
        match result {
            Ok(_) => {
                self.fillers.scanned = false;
                self.fillers.removed = Some(count);
            }
            Err(_) => self.fillers.problem = Some(Text::FillersFailed),
        }
    }
}
