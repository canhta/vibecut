//! Lends an area the open session and collects the notes it leaves for the frame.
use super::Request;
use crate::{egui, preferences::Preferences, services::lifecycle::EditorSession};
use std::time::Duration;

/// What an area reads while it edits: values and borrows of frame state it never owns.
pub(in crate::editor) struct Read<'a> {
    pub(in crate::editor) budget: &'a engine::ByteBudget,
    pub(in crate::editor) preferences: &'a Preferences,
    /// Transport position; `None` when the transport cannot report one.
    pub(in crate::editor) playhead: Option<time::Tick>,
    /// Timeline clip rectangles from the last painted frame.
    pub(in crate::editor) item_rects: &'a [(project::ItemId, egui::Rect)],
    /// The editor clock, for transient UI deadlines.
    pub(in crate::editor) now: Duration,
    /// Whether the frame allows an insertion right now.
    pub(in crate::editor) insertion_available: bool,
    /// Whether the caption bank accepts input right now.
    pub(in crate::editor) caption_bank_ready: bool,
}

/// One effect on frame-owned state that an area asks for; the frame applies it in order.
pub(in crate::editor) enum Note {
    Fail(String),
    Toast {
        text: String,
        detail: Option<String>,
    },
    ClearToast,
    /// Resync the preview with the session snapshot.
    Sync,
    /// The selection changed outside the timeline: drop the nested scopes and, when the
    /// property drafts described the old selection, the drafts too.
    ResetSelection {
        drafts: bool,
    },
    Preference {
        key: String,
        value: serde_json::Value,
        /// Record the edit in the app's own undo history.
        history: bool,
    },
    Widget {
        id: String,
        icon: Option<&'static str>,
        tooltip: Option<String>,
        response: egui::Response,
    },
    Request(Request),
}

/// The open session plus a read snapshot, lent to one area for one step. The area never
/// names the editor record; what it cannot do through the session it leaves as a note.
pub(in crate::editor) struct Port<'a> {
    pub(in crate::editor) session: &'a mut EditorSession,
    pub(in crate::editor) read: Read<'a>,
    notes: Vec<Note>,
}

impl<'a> Port<'a> {
    pub(in crate::editor) fn new(session: &'a mut EditorSession, read: Read<'a>) -> Self {
        Self {
            session,
            read,
            notes: Vec::new(),
        }
    }

    /// The notes the area left, in the order it left them.
    pub(in crate::editor) fn into_notes(self) -> Vec<Note> {
        self.notes
    }

    pub(in crate::editor) fn locale(&self) -> crate::i18n::Locale {
        self.read.preferences.locale
    }

    pub(in crate::editor) fn dark(&self) -> bool {
        self.read.preferences.dark
    }

    /// Localized text for the current locale.
    pub(in crate::editor) fn tr(&self, key: crate::i18n::Text) -> &'static str {
        crate::i18n::text(self.locale(), key)
    }

    /// The transport position, or the failure of reading it.
    pub(in crate::editor) fn playhead(&self) -> crate::Result<time::Tick> {
        self.read
            .playhead
            .ok_or(crate::Error::Invalid("playhead unavailable"))
    }

    /// Show a creator-facing failure on the editor's error line.
    pub(in crate::editor) fn fail(&mut self, message: impl Into<String>) {
        self.notes.push(Note::Fail(message.into()));
    }

    /// Show a creator-facing toast.
    pub(in crate::editor) fn toast(&mut self, text: &str, detail: Option<String>) {
        self.notes.push(Note::Toast {
            text: text.to_owned(),
            detail,
        });
    }

    pub(in crate::editor) fn clear_toast(&mut self) {
        self.notes.push(Note::ClearToast);
    }

    /// Resync the preview with the current project snapshot.
    pub(in crate::editor) fn sync(&mut self) {
        self.notes.push(Note::Sync);
    }

    /// The selection moved to something the nested scopes, and with `drafts` the property
    /// drafts, do not describe.
    pub(in crate::editor) fn reset_selection(&mut self, drafts: bool) {
        self.notes.push(Note::ResetSelection { drafts });
    }

    /// The one selection write. The session refuses a change while a gesture is open
    /// or when an item is gone; the refusal shows on the error line.
    pub(in crate::editor) fn select(
        &mut self,
        items: std::collections::BTreeSet<project::ItemId>,
    ) -> bool {
        match self.session.set_selection(items) {
            Ok(()) => true,
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }

    /// Apply one typed command; resync the preview when it committed a change.
    pub(in crate::editor) fn apply_result(
        &mut self,
        command: engine::Command,
    ) -> crate::Result<engine::Outcome> {
        let before = self.session.project().clone();
        match self.session.apply(command) {
            Ok(outcome) => {
                if outcome.changed || !std::sync::Arc::ptr_eq(&before, self.session.project()) {
                    self.sync();
                }
                Ok(outcome)
            }
            Err(error) => {
                self.fail(error.to_string());
                Err(error.into())
            }
        }
    }

    /// Write one preference without an app-history entry.
    pub(in crate::editor) fn store_preference(&mut self, key: &str, value: serde_json::Value) {
        self.notes.push(Note::Preference {
            key: key.to_owned(),
            value,
            history: false,
        });
    }

    /// Record one preference edit in the app's own history.
    pub(in crate::editor) fn set_preference(&mut self, key: &str, value: serde_json::Value) {
        self.notes.push(Note::Preference {
            key: key.to_owned(),
            value,
            history: true,
        });
    }

    /// Record one painted control, with the icon and tooltip its automation record shows.
    pub(in crate::editor) fn widget(
        &mut self,
        id: impl Into<String>,
        icon: Option<&'static str>,
        tooltip: Option<String>,
        response: &egui::Response,
    ) {
        self.notes.push(Note::Widget {
            id: id.into(),
            icon,
            tooltip,
            response: response.clone(),
        });
    }

    /// Queue a cross-area request; the frame drains it after this area returns.
    pub(in crate::editor) fn request(&mut self, request: Request) {
        self.notes.push(Note::Request(request));
    }
}
