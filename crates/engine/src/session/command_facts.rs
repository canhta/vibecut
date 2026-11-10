//! Declares the per-command facts that each command declaration carries.
use crate::Session;
use project::ItemParams;
use time::Tick;

/// Session condition under which a command is offered. Target and parameter
/// validation still happens at execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Always,
    /// A caption item exists and no transaction is open.
    CaptionItem,
    NoTransaction,
    GestureActive,
    /// A non-empty timeline and no open transaction.
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
            Self::CaptionItem => {
                !session.transaction_active()
                    && timeline.iter().any(|t| {
                        t.items
                            .iter()
                            .any(|i| matches!(i.content, ItemParams::Caption(_)))
                    })
            }
            Self::NoTransaction => !session.transaction_active(),
            Self::GestureActive => session.gesture_active(),
            Self::PlayableTimeline => {
                !session.transaction_active() && session.project().duration() > Tick::ZERO
            }
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
}

/// Every fact one declaration line adds to its command.
#[derive(Clone, Copy, Debug)]
pub(super) struct Facts {
    pub(super) available: Availability,
    pub(super) grant: Option<GrantClass>,
    pub(super) curated: bool,
    pub(super) lifecycle: Option<Lifecycle>,
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
