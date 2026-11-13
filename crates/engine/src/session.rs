//! Owns editable session state through atomic commands.
mod clipboard;
pub use clipboard::{
    AttributeGroup, ItemClipboard, ItemPasteSource, KeyframeClipboard, KeyframePasteSource,
};
mod failures;
mod fork;
pub use fork::{ChangeSummary, Fork};
mod revision;
mod state;
pub(crate) mod wire;
pub use failures::{BrandKitError, FontError, MusicError, SessionError, SoundError, StickerError};
pub(crate) use state::identity_entropy;
pub use state::{JobTarget, Session};
pub use wire::ChapterSidecars;
pub(crate) use wire::budget;
mod command_facts;
mod commands;
pub use command_facts::{Availability, CollectionLimit, GrantClass, Lifecycle};
use commands::EditCommand;
pub use commands::{
    AnimationPreset, AnimationSlot, CaptionPreset, CaptionStyleApply, ClipAnimations, Command,
    CommandKind, CommandSpec, CreatedId, CutTarget, Effect, EmphasisMode, Example, Fixture,
    ITEM_PARAMS, ItemParam, ItemParamPatch, ItemPlacement, KeyframeRef, Move, NewItem, Outcome,
    ParamBounds, ParamParseError, ParamTarget, ParamUnit, Placement, TrackPlacement, TrimEdge,
    declaration_order_error, item_param, lifecycle, registry,
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
pub use edit::{AssetUsage, DEFAULT_BOOKMARK_COLOR, EditError};
pub(crate) use edit::{DEFAULT_DURATION as DEFAULT_ANIMATION_DURATION, sample_animations};
