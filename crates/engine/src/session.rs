//! Owns editable session state through atomic commands.
mod failures;
mod revision;
mod state;
pub(crate) mod wire;
pub use failures::{FontError, SoundError};
pub use state::{JobTarget, Session};
pub use wire::ChapterSidecars;
pub(crate) use wire::budget;
mod command_facts;
mod commands;
pub use command_facts::{Availability, GrantClass, Lifecycle};
use commands::EditCommand;
pub use commands::{
    Command, CommandKind, CommandSpec, CreatedId, Effect, Example, Fixture, ItemParamPatch,
    ItemPlacement, KeyframeRef, Move, NewItem, Outcome, Placement, TrackPlacement, TrimEdge,
    declaration_order_error, lifecycle, registry,
};
mod edit;
pub(crate) mod filler;
pub(crate) mod transcript;
pub use transcript::{CueId, TranscriptCue, TranscriptView, TranscriptWord, WordId};
mod transition_rules;
pub(crate) mod transitions {
    //! Maintains video transitions across candidate edits.
    pub(crate) use super::transition_rules::{cuts, find, move_span, revalidate, unlocked};
}
pub use edit::DEFAULT_BOOKMARK_COLOR;
pub(crate) use edit::check_budget;
