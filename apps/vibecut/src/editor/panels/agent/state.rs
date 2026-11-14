//! Holds the agent chat's state across frames: consent, the running turn and the pending changes.
use super::{
    backend::{Backend, Environment},
    history,
    mention::Mention,
    transcript::{self, Counts, Decision, Entry},
    turn::{self, Done, Message, PollResult, Running},
};
use crate::{egui, i18n::Text};
use agent::{Provider, TurnEnd, new_conversation};
use ai::{
    Money,
    text::conversation::{
        AgentChatConsent, AgentChatLimits, AgentChatScope, Conversation, DataKind,
    },
};
use engine::{Failure as _, Fork, Session, control::ErrorCode};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

/// What the consent sheet binds: the service, the kinds of data and the spend limit.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Settings {
    pub(super) provider: Provider,
    pub(super) names: bool,
    pub(super) transcript: bool,
    pub(super) frames: bool,
    pub(super) limit_usd: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: Provider::Anthropic,
            names: false,
            transcript: false,
            // Frames never leave the machine unless the creator turns them on.
            frames: false,
            limit_usd: 1.0,
        }
    }
}

/// The changes of a finished turn that wait for Apply or Discard.
pub(super) struct Pending {
    /// The fork holding the edits; gone once an Apply was attempted.
    pub(super) fork: Option<Fork>,
    pub(super) counts: Counts,
    /// The turn ended by itself; otherwise these are the changes made before it stopped.
    pub(super) complete: bool,
    /// The live project changed after the fork: Apply refused and Retry runs the request again.
    pub(super) stale: bool,
}

/// What the frame must resync after an Apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Applied {
    Nothing,
    Changed,
    Stale,
    Failed,
}

/// Everything the panel asks the frame to do.
pub(in crate::editor) enum Action {
    /// A painted control, recorded for automation.
    Widget(String, egui::Response),
    Send {
        display: String,
        sent: String,
    },
    Accept,
    Stop,
    Apply,
    Discard,
    Retry,
    Preview(bool),
    Clear,
}

/// The chat panel's state. The frame owns one per editor.
pub(in crate::editor) struct State {
    pub(super) open: bool,
    pub(super) backend: Arc<dyn Backend>,
    history_dir: Option<PathBuf>,
    project: u64,
    pub(super) entries: Vec<Entry>,
    pub(super) input: String,
    /// Mentions picked into the input, resolved to ids when it is sent.
    pub(super) mentions: Vec<Mention>,
    pub(super) settings: Settings,
    /// The consent sheet is showing, for the first message or after a change.
    pub(super) sheet: bool,
    pub(super) consent: Option<AgentChatConsent>,
    conversation: Option<Conversation>,
    running: Option<Running>,
    pub(super) pending: Option<Pending>,
    pub(super) previewing: bool,
    /// A prompt that starts the first turn once the consent is granted (Home's card).
    pub(super) queued: Option<String>,
    pub(super) spent_micros: u64,
    pub(super) notice: Option<Text>,
    last_prompt: Option<String>,
    pub(super) ctx: Option<egui::Context>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            backend: Arc::new(Environment),
            history_dir: None,
            project: 0,
            entries: Vec::new(),
            input: String::new(),
            mentions: Vec::new(),
            settings: Settings::default(),
            sheet: true,
            consent: None,
            conversation: None,
            running: None,
            pending: None,
            previewing: false,
            queued: None,
            spent_micros: 0,
            notice: None,
            last_prompt: None,
            ctx: None,
        }
    }
}

/// How the panel reads at one moment, for automation and tests.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentView {
    pub open: bool,
    /// `no_key`, `consent`, `ready`, `running` or `decide`.
    pub stage: &'static str,
    /// Tool steps of the transcript with their outcome; `None` while running.
    pub steps: Vec<(String, Option<bool>)>,
    pub replies: Vec<String>,
    /// How the last turn ended early, as its history tag.
    pub ended: Option<String>,
    /// Items touched by the pending changes.
    pub changed: Option<u32>,
    pub previewing: bool,
    pub stale: bool,
    /// Lines of the transcript, which is also what the history keeps.
    pub history: usize,
    pub spent_micros: u64,
    pub queued: bool,
}

impl State {
    /// Lend the provider seam; tests lend a scripted one.
    pub(in crate::editor) fn set_backend(&mut self, backend: Arc<dyn Backend>) {
        self.backend = backend;
    }

    /// Keep this project's transcript under `dir` and show what it saved before.
    pub(in crate::editor) fn set_history(&mut self, dir: PathBuf, project: u64) {
        self.entries = history::load(&dir, project);
        self.history_dir = Some(dir);
        self.project = project;
    }

    pub(in crate::editor) fn is_open(&self) -> bool {
        self.open
    }

    pub(in crate::editor) fn open(&mut self) {
        self.open = true;
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.previewing = false;
    }

    /// Open the panel and start with `prompt` as soon as the consent allows.
    pub(in crate::editor) fn queue_prompt(&mut self, prompt: String) {
        self.open = true;
        self.queued = Some(prompt);
    }

    /// The fork the player and timeline show while Preview is on.
    pub(in crate::editor) fn preview_session(&self) -> Option<&Session> {
        if !self.previewing {
            return None;
        }
        self.pending
            .as_ref()
            .and_then(|pending| pending.fork.as_ref())
            .map(Fork::session)
    }

    /// Whether the areas currently show the agent's changes instead of the project.
    pub(in crate::editor) fn previewing(&self) -> bool {
        self.preview_session().is_some()
    }

    pub(super) fn running(&self) -> bool {
        self.running.is_some()
    }

    /// Whether any provider has a key configured.
    pub(super) fn any_key(&self) -> bool {
        [Provider::Anthropic, Provider::OpenAi]
            .into_iter()
            .any(|provider| self.backend.has_key(provider))
    }

    pub(super) fn stage(&self) -> &'static str {
        if self.running.is_some() {
            "running"
        } else if self.pending.is_some() {
            "decide"
        } else if self.consent.is_none() || self.sheet {
            if self.any_key() { "consent" } else { "no_key" }
        } else {
            "ready"
        }
    }

    pub(in crate::editor) fn view(&self) -> AgentView {
        AgentView {
            open: self.open,
            stage: self.stage(),
            steps: self
                .entries
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Step { name, ok, .. } => Some((name.clone(), *ok)),
                    _ => None,
                })
                .collect(),
            replies: self
                .entries
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Assistant { text } => Some(text.clone()),
                    _ => None,
                })
                .collect(),
            ended: self.entries.iter().rev().find_map(|entry| match entry {
                Entry::Ended { reason } => Some(reason.clone()),
                _ => None,
            }),
            changed: self.pending.as_ref().map(|pending| {
                let counts = &pending.counts;
                counts.added
                    + counts.removed
                    + counts.moved
                    + counts.trimmed
                    + counts.retimed
                    + counts.edited
                    + counts.captions
            }),
            previewing: self.previewing,
            stale: self.pending.as_ref().is_some_and(|pending| pending.stale),
            history: self.entries.len(),
            spent_micros: self.spent_micros,
            queued: self.queued.is_some(),
        }
    }

    fn persist(&self) {
        if let Some(dir) = &self.history_dir
            && history::save(dir, self.project, &self.entries).is_err()
        {
            tracing::warn!("the agent history could not be saved");
        }
    }

    /// Open the model for the settings, grant the consent they describe and start a
    /// conversation under it. A refusal leaves the sheet up and sends nothing.
    pub(super) fn accept(&mut self) -> bool {
        let settings = self.settings.clone();
        let mut kinds = BTreeSet::from([DataKind::ProjectSummary]);
        for (on, kind) in [
            (settings.names, DataKind::ClipNames),
            (settings.transcript, DataKind::TranscriptText),
            (settings.frames, DataKind::Frames),
        ] {
            if on {
                kinds.insert(kind);
            }
        }
        let micros = (settings.limit_usd * 1_000_000.0).round().max(0.0) as u64;
        let built = self
            .backend
            .open(settings.provider)
            .ok()
            .zip(Money::new("USD", micros).ok())
            .and_then(|(chat, limit)| {
                let scope = AgentChatScope {
                    provider: chat.config().clone(),
                    kinds,
                    limit,
                };
                let consent = AgentChatConsent::grant(scope).ok()?;
                let conversation =
                    new_conversation(chat, &consent, AgentChatLimits::default()).ok()?;
                Some((consent, conversation))
            });
        match built {
            Some((consent, conversation)) => {
                self.consent = Some(consent);
                self.conversation = Some(conversation);
                self.spent_micros = 0;
                self.sheet = false;
                self.notice = None;
                true
            }
            None => {
                self.notice = Some(Text::AgentStartFailed);
                false
            }
        }
    }

    /// Refuse the sheet: nothing is sent and a waiting prompt is dropped.
    pub(super) fn decline(&mut self) {
        self.queued = None;
        self.open = false;
    }

    /// A conversation under the granted consent, rebuilt after a history clear.
    fn ensure_conversation(&mut self) -> bool {
        if self.conversation.is_some() {
            return true;
        }
        let Some(consent) = &self.consent else {
            return false;
        };
        let rebuilt = Provider::parse(consent.scope().provider.id().as_str())
            .and_then(|provider| self.backend.open(provider).ok())
            .and_then(|chat| new_conversation(chat, consent, AgentChatLimits::default()).ok());
        self.conversation = rebuilt;
        if self.conversation.is_none() {
            self.notice = Some(Text::AgentStartFailed);
        }
        self.conversation.is_some()
    }

    /// Fork `session` and start a turn on a worker. `display` is the user's line for the
    /// transcript (a retry adds none); `sent` is what the model reads.
    pub(super) fn begin(
        &mut self,
        session: &Session,
        budget: &engine::ByteBudget,
        display: Option<String>,
        sent: String,
    ) {
        if self.running.is_some() || self.pending.is_some() || self.consent.is_none() {
            return;
        }
        if !self.ensure_conversation() {
            return;
        }
        let Ok(fork) = session.fork() else {
            self.notice = Some(Text::AgentStartFailed);
            return;
        };
        let Some(conversation) = self.conversation.take() else {
            return;
        };
        match turn::spawn(
            fork,
            sent.clone(),
            conversation,
            budget.clone(),
            self.ctx.clone(),
        ) {
            Some(running) => {
                if let Some(text) = display {
                    self.entries.push(Entry::User { text });
                }
                self.last_prompt = Some(sent);
                self.running = Some(running);
                self.notice = None;
                self.persist();
            }
            None => self.notice = Some(Text::AgentStartFailed),
        }
    }

    /// Ask the running turn to stop; it ends with its fork intact.
    pub(super) fn stop(&mut self) {
        if let Some(running) = &self.running {
            running.cancel();
        }
    }

    /// Take what the worker sent since the last frame.
    pub(in crate::editor) fn tick(&mut self) {
        while let Some(running) = self.running.as_mut() {
            match running.poll() {
                None => return,
                Some(PollResult::Message(Message::Event(event))) => {
                    transcript::fold(&mut self.entries, &event);
                }
                Some(PollResult::Message(Message::Done(done))) => {
                    self.running = None;
                    self.finish(*done);
                }
                Some(PollResult::Lost) => {
                    self.running = None;
                    self.notice = Some(Text::AgentStartFailed);
                }
            }
        }
    }

    fn finish(&mut self, done: Done) {
        let Done { turn, conversation } = done;
        self.spent_micros = conversation.ledger().spent_micros();
        self.conversation = Some(conversation);
        match turn {
            Some(turn) => {
                // A turn that changed nothing has no card: its answer is the text above.
                if !turn.fork.is_empty() {
                    self.pending = Some(Pending {
                        counts: Counts::of(&turn.fork.summary()),
                        complete: turn.end == TurnEnd::Done,
                        stale: false,
                        fork: Some(turn.fork),
                    });
                }
            }
            None => self.notice = Some(Text::AgentStartFailed),
        }
        self.persist();
    }

    /// Replay the fork's edits on the live session as one undo step.
    pub(super) fn apply_changes(&mut self, session: &mut Session) -> Applied {
        let Some(mut pending) = self.pending.take() else {
            return Applied::Nothing;
        };
        self.previewing = false;
        let Some(fork) = pending.fork.take() else {
            // Only a stale card has no fork; it stays for Retry or Discard.
            self.pending = Some(pending);
            return Applied::Nothing;
        };
        match session.apply_fork(fork) {
            Ok(_) => {
                self.entries.push(Entry::Changes {
                    counts: pending.counts,
                    decision: Decision::Applied,
                });
                self.persist();
                Applied::Changed
            }
            Err(error) if error.code() == ErrorCode::StaleRevision => {
                pending.stale = true;
                self.pending = Some(pending);
                Applied::Stale
            }
            Err(_) => {
                self.entries.push(Entry::Changes {
                    counts: pending.counts,
                    decision: Decision::Discarded,
                });
                self.notice = Some(Text::AgentApplyFailed);
                self.persist();
                Applied::Failed
            }
        }
    }

    /// Drop the fork; the live project was never touched.
    pub(super) fn discard(&mut self) {
        self.previewing = false;
        if let Some(pending) = self.pending.take() {
            self.entries.push(Entry::Changes {
                counts: pending.counts,
                decision: Decision::Discarded,
            });
            self.persist();
        }
    }

    /// Run the last request again on a new fork of the current project.
    pub(super) fn retry(&mut self, session: &Session, budget: &engine::ByteBudget) {
        let Some(prompt) = self.last_prompt.clone() else {
            return;
        };
        self.discard();
        self.begin(session, budget, None, prompt);
    }

    /// Forget the transcript and the conversation; the consent stays.
    pub(super) fn clear(&mut self) {
        if self.running.is_some() || self.pending.is_some() {
            return;
        }
        self.entries.clear();
        self.conversation = None;
        self.spent_micros = 0;
        if let Some(dir) = &self.history_dir
            && history::clear(dir, self.project).is_err()
        {
            tracing::warn!("the agent history could not be removed");
        }
    }
}
