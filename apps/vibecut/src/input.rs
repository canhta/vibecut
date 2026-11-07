//! Chords, binding storage, keymap resolution and global key dispatch.
//!
//! The action ids and labels themselves are owned by each window's action
//! table; this foundation only understands ids, defaults and bindability.
mod chord;
mod dispatch;
mod keybindings;
mod keymap;

pub use chord::{Chord, ChordError, Mods, Platform};
pub use dispatch::{BlockedReason, Dispatch, DispatchContext};
#[cfg(test)]
pub(crate) use keybindings::valid_action_id;
pub use keybindings::{
    KEYBINDING_FORMAT, KEYBINDING_VERSION, KEYBINDINGS_RESET_COMMAND, KEYBINDINGS_SET_COMMAND,
    KeybindingError, KeybindingIssue, KeybindingPreferences, KeybindingProfile, MAX_ACTIONS,
    MAX_BINDINGS_PER_ACTION, MAX_CHORD_TEXT, MAX_PROFILE_BYTES,
};
pub use keymap::{DispatchInfo, Keymap};
