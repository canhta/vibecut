//! Declares the per-command facts that each command declaration carries.
use super::SessionError;
use super::wire::ResultKind;
use crate::Session;
use project::ItemParams;
use serde::Serialize;
use time::Tick;

/// Session condition under which a command is offered. Target and parameter
/// validation still happens at execution. A declaration states separately whether
/// it also refuses while a transaction is open (`idle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Always,
    /// A caption item exists.
    CaptionItem,
    GestureActive,
    /// A non-empty timeline.
    PlayableTimeline,
    CanUndo,
    CanRedo,
    /// At least one media asset.
    Assets,
    /// An unlocked video or audio item.
    RetimableItem,
    /// An unlocked overlay or audio track.
    RemovableTrack,
    /// An unlocked item that carries visual parameters.
    VisualItem,
    /// An unlocked video item.
    VideoItem,
    /// Any unlocked item.
    UnlockedItem,
}
impl Availability {
    pub fn holds(self, session: &Session) -> bool {
        let timeline = &session.project().data().scene().timeline;
        match self {
            Self::Always => true,
            Self::CaptionItem => timeline.iter().any(|t| {
                t.items
                    .iter()
                    .any(|i| matches!(i.content, ItemParams::Caption(_)))
            }),
            Self::GestureActive => session.gesture_active(),
            Self::PlayableTimeline => session.project().duration() > Tick::ZERO,
            Self::CanUndo => session.can_undo(),
            Self::CanRedo => session.can_redo(),
            Self::Assets => !session.project().data().assets.is_empty(),
            Self::RetimableItem => timeline.iter().any(|t| {
                !t.locked
                    && t.items
                        .iter()
                        .any(|i| matches!(i.content, ItemParams::Video(_) | ItemParams::Audio(_)))
            }),
            Self::RemovableTrack => timeline
                .overlays
                .iter()
                .chain(&timeline.audio)
                .any(|t| !t.locked),
            Self::VisualItem => timeline.iter().any(|t| {
                !t.locked
                    && t.items
                        .iter()
                        .any(|i| !matches!(i.content, ItemParams::Audio(_)))
            }),
            Self::VideoItem => timeline.iter().any(|t| {
                !t.locked
                    && t.items
                        .iter()
                        .any(|i| matches!(i.content, ItemParams::Video(_)))
            }),
            Self::UnlockedItem => timeline.iter().any(|t| !t.locked && !t.items.is_empty()),
        }
    }

    /// The typed refusal of an unavailable command: history and gesture commands keep
    /// the cause they always had, every other condition is `Unavailable`.
    pub(crate) fn check(self, session: &Session) -> Result<(), SessionError> {
        if self.holds(session) {
            return Ok(());
        }
        Err(match self {
            Self::GestureActive => SessionError::NoGesture,
            Self::CanUndo | Self::CanRedo => SessionError::NoHistory,
            _ => SessionError::Unavailable,
        })
    }
}

/// Who may run a command over an agent transport; a transport maps it to its own
/// trusted grants and never infers permission from names or effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantClass {
    /// Any admitted agent caller.
    Agent,
    /// Only into the output directory the human selected, never overwriting.
    Output,
    /// A person or trusted application only: it reads source paths, owns the
    /// project library or controls the UI gesture.
    Human,
}

/// A command that the library or persistence owner executes, not the edit candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    Create,
    Duplicate,
    Delete,
    Restore,
    Purge,
    Thumbnail,
    Save,
    /// Publishes a copy of the open project under a fresh identity.
    SaveAs,
    /// Writes the open project and its resources as a bundle directory.
    ExportBundle,
    /// Extracts a bundle into a new project directory.
    ImportBundle,
}

/// A collection parameter of a command and the most elements it may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct CollectionLimit {
    pub field: &'static str,
    pub max: usize,
}

/// Every fact one declaration line adds to its command.
#[derive(Clone, Copy, Debug)]
pub(super) struct Facts {
    pub(super) available: Availability,
    /// Refused while a gesture or transaction is open.
    pub(super) idle: bool,
    pub(super) grant: Option<GrantClass>,
    pub(super) curated: bool,
    pub(super) lifecycle: Option<Lifecycle>,
    /// The command keeps the transitions of the timeline consistent itself, so an
    /// edit does not snapshot the cuts around it.
    pub(super) owns_transitions: bool,
    /// The `ControlResult` variants its dispatch may return.
    pub(super) results: &'static [ResultKind],
}
impl Facts {
    /// The one availability rule: the transaction clause, then the session condition.
    pub(super) fn admit(&self, session: &Session) -> Result<(), SessionError> {
        if self.idle && session.transaction_active() {
            return Err(SessionError::TransactionActive);
        }
        self.available.check(session)
    }
}

/// Elements of a collection parameter, whichever container the command declares it in.
pub(super) trait Collection {
    fn element_count(&self) -> usize;
}
impl<T> Collection for Vec<T> {
    fn element_count(&self) -> usize {
        self.len()
    }
}
impl<T> Collection for Option<Vec<T>> {
    fn element_count(&self) -> usize {
        self.as_ref().map_or(0, Vec::len)
    }
}

impl super::Effect {
    /// Effects whose commands share one grant class; `None` makes the declaration
    /// state it, so a command can never be denied or allowed by omission.
    pub(super) const fn default_grant(self) -> Option<GrantClass> {
        match self {
            Self::Edit | Self::Read => Some(GrantClass::Agent),
            Self::FilePublish => Some(GrantClass::Output),
            Self::SessionControl | Self::JobControl => None,
        }
    }
}
