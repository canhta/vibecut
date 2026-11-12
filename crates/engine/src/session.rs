//! Owns editable session state through atomic commands.
mod clipboard;
pub use clipboard::{KeyframeClipboard, KeyframePasteSource};
mod failures;
mod revision;
mod state;
pub(crate) mod wire;
pub use failures::{FontError, MusicError, SessionError, SoundError};
pub(crate) use state::identity_entropy;
pub use state::{JobTarget, Session};
pub use wire::ChapterSidecars;
pub(crate) use wire::budget;
mod command_facts;
mod commands;
pub use command_facts::{Availability, GrantClass, Lifecycle};
use commands::EditCommand;
pub use commands::{
    CaptionPreset, CaptionStyleApply, Command, CommandKind, CommandSpec, CreatedId, Effect,
    Example, Fixture, ItemParamPatch, ItemPlacement, KeyframeRef, Move, NewItem, Outcome,
    Placement, TextAnimationPreset, TrackPlacement, TrimEdge, declaration_order_error, lifecycle,
    registry,
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
pub(crate) use edit::Result;
pub(crate) use edit::check_budget;
pub(crate) use edit::merge_reframe;
pub use edit::{AssetUsage, DEFAULT_BOOKMARK_COLOR, EditError};
