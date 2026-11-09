//! Owns editable session state through atomic commands.
mod failures;
mod revision;
mod state;
pub(crate) mod wire;
pub use failures::{FontError, SoundError};
pub use state::{JobTarget, Session};
pub(crate) use wire::budget;
mod command_facts;
mod commands;
pub use command_facts::{Availability, GrantClass, Lifecycle};
pub use commands::{
    Command, CommandKind, CommandSpec, CreatedId, Effect, Example, Fixture, ItemParamPatch,
    ItemPlacement, KeyframeRef, Move, NewItem, Outcome, Placement, TrackPlacement, TrimEdge,
    lifecycle, registry,
};
mod edit;
mod transition_rules;
pub(crate) mod transitions {
    //! Maintains video transitions across candidate edits.
    pub(crate) use super::transition_rules::{cuts, find, move_span, revalidate, unlocked};
}
pub use edit::DEFAULT_BOOKMARK_COLOR;
pub(crate) use edit::check_budget;
