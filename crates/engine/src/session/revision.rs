//! Advances the session revision and records each change in the ledger.
use super::wire::{Payload, Revision};
use crate::{EditError, Result, Session};
use std::sync::Arc;

impl Session {
    pub fn revision(&self) -> Revision {
        (*self.revision).clone()
    }

    pub fn session_id(&self) -> &str {
        &self.identity.0
    }

    pub(crate) fn invalidate_revision(&mut self, committed: bool) -> Result<()> {
        self.generation = self.generation.checked_add(1).ok_or(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ))?;
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
}
