//! Advances the session revision and records each change in the ledger.
use super::wire::{Event, Payload, Revision};
use crate::{EditError, Result, Session};
use std::{collections::VecDeque, sync::Arc};

impl Session {
    pub fn revision(&self) -> Revision {
        (*self.revision).clone()
    }

    pub fn session_id(&self) -> &str {
        &self.identity.0
    }

    pub(crate) fn invalidate_revision(&mut self, committed: bool) -> Result<()> {
        self.generation = self.generation.checked_add(1).ok_or(EditError::Limit)?;
        self.revision = Arc::new(Revision(format!("{}:{}", self.identity.0, self.generation)));
        self.ledger.push(
            &self.identity.0,
            self.revision(),
            if committed {
                Payload::ProjectChanged {
                    full_snapshot_required: true,
                }
            } else {
                Payload::RevisionChanged
            },
        );
        Ok(())
    }

    pub(crate) fn control_mark(&self) -> (u64, VecDeque<Arc<Event>>) {
        (self.ledger.sequence, self.ledger.events.clone())
    }

    pub(crate) fn restore_control_mark(&mut self, mark: (u64, VecDeque<Arc<Event>>)) {
        self.ledger.sequence = mark.0;
        self.ledger.events = mark.1;
    }
}
