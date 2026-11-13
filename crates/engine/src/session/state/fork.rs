//! Forks a session and applies a fork's recorded commands back as one undo step.
use super::Session;
use crate::session::fork::{Fork, Logged};
use crate::session::wire::ControlError;
use crate::session::{EditError, Result, SessionError};
use crate::{Command, Outcome};

impl Session {
    /// Copy the project, ID counter, ripple policy, clipboards and view state into an
    /// isolated session with no history and an empty command log. Refused while a gesture,
    /// preview or group is open, because its project is not yet a committed revision.
    pub fn fork(&self) -> Result<Fork> {
        if self.transaction_active() {
            return Err(SessionError::TransactionActive.into());
        }
        let mut session = Self::new(self.state.project.as_ref().clone(), self.limit)?;
        session.state.next_id = self.state.next_id;
        session.clipboard = self.clipboard.clone();
        session.item_clipboard = self.item_clipboard.clone();
        session.metadata = self.metadata.clone();
        session.protected = self.protected.clone();
        session.ripple = self.ripple;
        session.recorded = Some(Vec::new());
        Ok(Fork {
            session,
            base: self.state.project.clone(),
            base_revision: self.revision(),
        })
    }

    /// Replay the commands that changed the fork as one undo step and one change event.
    ///
    /// Refuses `StaleRevision` unless this session is at the revision the fork was taken at,
    /// which also rules out any other session of the same project. An empty log is a no-op
    /// with no undo entry. A failing command, or a replay that does not end at the fork's
    /// project, restores the project, history and pending change; the revision still advances,
    /// as for every rolled-back group, so job targets captured meanwhile go stale.
    pub fn apply_fork(&mut self, fork: Fork) -> Result<Outcome> {
        if self.transaction_active() {
            return Err(SessionError::TransactionActive.into());
        }
        if fork.base_revision != self.revision() {
            return Err(ControlError::StaleRevision.into());
        }
        let mut forked = fork.session;
        let log = forked.recorded.take().unwrap_or_default();
        let mut outcome = Outcome {
            changed: false,
            created: Vec::new(),
            placements: Vec::new(),
            skipped: Vec::new(),
            closed_span: false,
        };
        if log.is_empty() {
            return Ok(outcome);
        }
        let target = forked.state.project;
        self.transaction(|live| {
            for group in log {
                // The fork ran each entry on its own: trims never carried cue sources from
                // one entry to the next, but a grouped entry shared them among its children.
                live.state.trim_sources.clear();
                for Logged {
                    command,
                    wire,
                    next_id,
                } in group
                {
                    live.state.next_id = next_id;
                    let applied = live.boundary(|s| s.commit(command, wire, s.ripple))?;
                    outcome.created.extend(applied.created);
                    outcome.placements.extend(applied.placements);
                    outcome.skipped.extend(applied.skipped);
                    outcome.closed_span |= applied.closed_span;
                }
            }
            if *live.state.project != *target {
                return Err(EditError::Session(SessionError::ForkDiverged));
            }
            Ok::<(), EditError>(())
        })?;
        outcome.changed = true;
        Ok(outcome)
    }

    /// Log `command` when this session is a fork and the command changed the project.
    pub(super) fn record(
        &mut self,
        command: Option<Command>,
        wire: bool,
        next_id: Option<u64>,
        outcome: &Outcome,
    ) {
        if outcome.changed
            && let (Some(log), Some(command)) = (&mut self.recorded, command)
        {
            log.push(vec![Logged {
                command,
                wire,
                next_id,
            }]);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "validated test fixtures")]
    use super::*;
    use std::sync::Arc;

    fn add_track() -> Command {
        Command::AddTrack {
            placement: crate::TrackPlacement::Auto,
            name: "".into(),
            kind: project::TrackKind::Video,
        }
    }
    fn live() -> Session {
        Session::new(
            project::Project::empty(project::ProjectId(0), "".into()).unwrap(),
            4,
        )
        .unwrap()
    }
    fn forge(fork: &mut Fork, command: Command) {
        let next_id = fork.session().state.next_id;
        fork.session_mut()
            .recorded
            .as_mut()
            .unwrap()
            .push(vec![Logged {
                command,
                wire: false,
                next_id,
            }]);
    }

    /// Forged inline because a fork's own session can never log an entry its parent refuses:
    /// a child the live project refuses, and a replay that lands elsewhere.
    #[test]
    fn a_failing_child_or_a_diverging_replay_restores_everything() {
        for forged in [
            Command::Delete {
                items: vec![project::ItemId(99)],
            },
            add_track(),
        ] {
            let mut live = live();
            let mut fork = live.fork().unwrap();
            forge(&mut fork, add_track());
            forge(&mut fork, forged);
            let before = live.project().clone();
            let result = live.apply_fork(fork);
            assert!(result.is_err());
            assert!(Arc::ptr_eq(&before, live.project()));
            assert_eq!(live.history_len(), (0, 0));
            assert!(live.take_change().is_none());
        }
    }
}
