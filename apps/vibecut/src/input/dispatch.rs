//! Dispatch blocking reasons and the outcome of one key press.

use super::Chord;

/// Why global dispatch is inert. The editor still lets the focused widget see
/// the key event so text editing and Escape-to-blur keep working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedReason {
    Overlay,
    ProjectLoading,
    Recording,
    TextInput,
}

/// Everything outside the keymap that can block global dispatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DispatchContext {
    pub overlay_open: bool,
    pub project_loading: bool,
    pub recording: bool,
    pub text_input: bool,
    /// The in-app clipboard has an item or keyframe entry. Presence, not payload,
    /// is the dispatch contract; p1-parity/20 owns the payload.
    pub clipboard_entry: bool,
}

impl DispatchContext {
    pub fn blocked(&self) -> Option<BlockedReason> {
        if self.overlay_open {
            Some(BlockedReason::Overlay)
        } else if self.project_loading {
            Some(BlockedReason::ProjectLoading)
        } else if self.recording {
            Some(BlockedReason::Recording)
        } else if self.text_input {
            Some(BlockedReason::TextInput)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch {
    Blocked(BlockedReason),
    NoMatch,
    FellThrough(&'static str),
    Unavailable {
        action: &'static str,
        owner: &'static str,
    },
    Action {
        action: &'static str,
        chord: Chord,
    },
}
