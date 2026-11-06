//! Owns editable session state through atomic commands.
mod revision;
mod state;
pub(crate) mod wire;
pub use state::{JobTarget, Session};
pub(crate) use wire::budget;
mod commands;
pub use commands::{
    Command, CommandKind, CommandSpec, CreatedId, Effect, Example, Fixture, ItemParamPatch,
    ItemPlacement, Move, NewItem, Outcome, Placement, TrackPlacement, registry,
};
mod edit;
mod transition_rules;
pub(crate) mod transitions {
    //! Maintains video transitions across candidate edits.
    pub(crate) use super::transition_rules::{cuts, find, move_span, revalidate, unlocked};
}
pub(crate) use edit::check_budget;
