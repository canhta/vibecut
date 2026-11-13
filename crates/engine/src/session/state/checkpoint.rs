//! Captures what one command boundary may change so a refusal restores it.
use super::{Session, State};
use crate::session::wire::{Event, Revision};
use std::{collections::VecDeque, sync::Arc};

/// Session state at entry to one command boundary.
///
/// Every `Session` field is classified here by exhaustive destructuring, so a
/// new field does not compile until it is restored or excluded. Excluded:
/// `protected` (set only by `protect_paths`, never by a command), `identity`
/// and `limit` (fixed at construction), `generation` (monotonic: lifetimes a
/// refused command opened stay retired) and `grouping` (`transaction` sets and
/// clears it around its own closure), `recorded` (a fork's command log, appended only after
/// a boundary succeeded and truncated by `transaction` when its group fails).
///
/// A boundary restores item IDs with the rest of `state`: a refused command
/// returned no outcome, so nothing observed the IDs it allocated. Gestures and
/// transactions use [`State::rewind`] instead, which keeps the ID counter
/// because their children already returned IDs to the caller.
pub(super) struct Checkpoint {
    clipboard: Option<Arc<crate::KeyframeClipboard>>,
    item_clipboard: Option<Arc<crate::ItemClipboard>>,
    metadata: project::Metadata,
    revision: Arc<Revision>,
    ledger_sequence: u64,
    ledger_events: VecDeque<Arc<Event>>,
    state: State,
    gesture: Option<State>,
    timeline_preview: bool,
    ripple: bool,
}

impl Checkpoint {
    pub(super) fn take(session: &Session) -> Self {
        let Session {
            clipboard,
            item_clipboard,
            metadata,
            protected: _,
            identity: _,
            revision,
            generation: _,
            ledger,
            state,
            limit: _,
            recorded: _,
            gesture,
            timeline_preview,
            grouping: _,
            asset_export_sources: _,
            loudness_sources: _,
            ripple,
        } = session;
        Self {
            clipboard: clipboard.clone(),
            item_clipboard: item_clipboard.clone(),
            metadata: metadata.clone(),
            revision: revision.clone(),
            ledger_sequence: ledger.sequence,
            ledger_events: ledger.events.clone(),
            state: state.clone(),
            gesture: gesture.clone(),
            timeline_preview: *timeline_preview,
            ripple: *ripple,
        }
    }

    pub(super) fn identity_changed(&self, session: &Session) -> bool {
        self.state.project.data().id != session.state.project.data().id
            || self.state.project.data().current_scene_id
                != session.state.project.data().current_scene_id
    }
    /// The command replaced the project but left the revision it started with.
    pub(super) fn project_changed_at_same_revision(&self, session: &Session) -> bool {
        Arc::ptr_eq(&self.revision, &session.revision)
            && !Arc::ptr_eq(&self.state.project, &session.state.project)
    }

    pub(super) fn restore(self, session: &mut Session) {
        let Session {
            clipboard,
            item_clipboard,
            metadata,
            protected: _,
            identity: _,
            revision,
            generation: _,
            ledger,
            state,
            limit: _,
            recorded: _,
            gesture,
            timeline_preview,
            grouping: _,
            asset_export_sources: _,
            loudness_sources: _,
            ripple,
        } = session;
        *clipboard = self.clipboard;
        *item_clipboard = self.item_clipboard;
        *metadata = self.metadata;
        *revision = self.revision;
        ledger.sequence = self.ledger_sequence;
        ledger.events = self.ledger_events;
        *state = self.state;
        *gesture = self.gesture;
        *timeline_preview = self.timeline_preview;
        *ripple = self.ripple;
    }
}
