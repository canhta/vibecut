//! Resynchronizes control listeners with committed state.
use crate::jobs::{JobRunner, JobSnapshot};
use crate::session::wire::{
    ControlResult, Cursor, ErrorCode, Event, JobView, Payload, Response, ResponseValue, Snapshot,
};
use crate::{EditError, Result, Session};

impl From<JobSnapshot> for JobView {
    fn from(s: JobSnapshot) -> Self {
        Self {
            id: s.id,
            kind: s.kind,
            stage: s.stage,
            progress: s.progress,
            state: s.state,
            attempts: s.attempts,
            error: s.error.as_deref().map(ErrorCode::from_error),
            result: None,
            pauses: None,
            chapters: None,
            fillers: None,
        }
    }
}

impl Session {
    pub(super) fn push_control_event(&mut self, request_id: &str, response: &Response) {
        match &response.value {
            ResponseValue::Result(result) => {
                // Large read results use the response path; event consumers resync.
                if matches!(
                    result,
                    ControlResult::Edit { .. }
                        | ControlResult::JobAccepted { .. }
                        | ControlResult::CancelRequested { .. }
                ) {
                    self.ledger.push(
                        &self.identity.0,
                        self.revision(),
                        Payload::CommandResult {
                            request_id: request_id.to_owned(),
                            result: result.clone(),
                        },
                    );
                }
            }
            ResponseValue::Error(code) => self.ledger.push(
                &self.identity.0,
                self.revision(),
                Payload::CommandError {
                    request_id: request_id.to_owned(),
                    code: *code,
                },
            ),
        }
    }
}

impl Session {
    /// Atomically capture committed state and a listener watermark under Session
    /// ownership. Preview state is deliberately not presented as committed state.
    pub fn snapshot_and_subscribe(&mut self, runner: &JobRunner) -> Result<Snapshot> {
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.observe_jobs(runner);
        Ok(Snapshot {
            project: self.project().clone(),
            revision: self.revision(),
            jobs: runner.snapshots().into_iter().map(JobView::from).collect(),
            cursor: Cursor {
                session: self.session_id().into(),
                seq: self.ledger.sequence,
            },
        })
    }
}

impl Session {
    /// Pump from the same owning loop as JobRunner::tick. Does not drain another
    /// consumer's runner events or cancel work when a listener disappears.
    pub fn observe_jobs(&mut self, runner: &JobRunner) {
        let snapshots = runner.snapshots();
        self.ledger
            .jobs
            .retain(|id, _| snapshots.iter().any(|s| s.id == *id));
        for snapshot in snapshots {
            let view = JobView::from(snapshot);
            let changed = self.ledger.jobs.get(&view.id).is_none_or(|old| {
                old.state != view.state
                    || old.stage != view.stage
                    || old.attempts != view.attempts
                    || old.progress != view.progress
            });
            if changed {
                self.ledger.jobs.insert(view.id, view.clone());
                self.ledger.push(
                    &self.identity.0,
                    self.revision(),
                    Payload::JobChanged { snapshot: view },
                );
            }
        }
    }
}

impl Session {
    pub fn events(&self, cursor: &mut Cursor) -> Result<Vec<Event>> {
        if cursor.session != self.session_id()
            || cursor.seq > self.ledger.sequence
            || self.ledger.events.front().is_some_and(|e| {
                e.seq
                    .parse::<u64>()
                    .is_ok_and(|seq| cursor.seq.saturating_add(1) < seq)
            })
        {
            return Err(EditError::ResyncRequired);
        }
        let events = self
            .ledger
            .events
            .iter()
            .filter(|e| e.seq.parse::<u64>().is_ok_and(|seq| seq > cursor.seq))
            .map(|e| (**e).clone())
            .collect();
        cursor.seq = self.ledger.sequence;
        Ok(events)
    }
}
