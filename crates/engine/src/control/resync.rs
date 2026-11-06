//! Resynchronizes control listeners with committed state.
use super::{
    API_VERSION, ControlResult, Cursor, EditError, Event, JobRunner, JobView, MAX_EVENTS, Payload,
    Response, ResponseValue, Result, Revision, Session, Snapshot, State, VecDeque, registry_hash,
};
impl State {
    pub(crate) fn push(&mut self, session: &str, revision: Revision, payload: Payload) {
        // A saturated stream can never manufacture another sequence: all cursors
        // require resync. This is unreachable in a practical Session lifetime.
        let Some(next) = self.sequence.checked_add(1) else {
            self.events.clear();
            return;
        };
        self.sequence = next;
        self.events.push_back(std::sync::Arc::new(Event {
            api_version: API_VERSION,
            registry_hash: registry_hash().into(),
            session_id: session.into(),
            seq: next.to_string(),
            revision,
            payload,
        }));
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }
}
impl Session {
    pub(crate) fn control_mark(&self) -> (u64, VecDeque<std::sync::Arc<Event>>) {
        (self.control.sequence, self.control.events.clone())
    }
}

impl Session {
    pub(crate) fn restore_control_mark(&mut self, mark: (u64, VecDeque<std::sync::Arc<Event>>)) {
        self.control.sequence = mark.0;
        self.control.events = mark.1;
    }
}

impl Session {
    pub fn revision(&self) -> Revision {
        (*self.revision).clone()
    }
}

impl Session {
    pub fn session_id(&self) -> &str {
        &self.identity.0
    }
}

impl Session {
    pub(crate) fn invalidate_revision(&mut self, committed: bool) -> Result<()> {
        self.generation = self.generation.checked_add(1).ok_or(EditError::Limit)?;
        self.revision =
            std::sync::Arc::new(Revision(format!("{}:{}", self.identity.0, self.generation)));
        self.control.push(
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
                    self.control.push(
                        &self.identity.0,
                        self.revision(),
                        Payload::CommandResult {
                            request_id: request_id.to_owned(),
                            result: result.clone(),
                        },
                    );
                }
            }
            ResponseValue::Error(code) => self.control.push(
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
                seq: self.control.sequence,
            },
        })
    }
}

impl Session {
    /// Pump from the same owning loop as JobRunner::tick. Does not drain another
    /// consumer's runner events or cancel work when a listener disappears.
    pub fn observe_jobs(&mut self, runner: &JobRunner) {
        let snapshots = runner.snapshots();
        self.control
            .jobs
            .retain(|id, _| snapshots.iter().any(|s| s.id == *id));
        for snapshot in snapshots {
            let view = JobView::from(snapshot);
            let changed = self.control.jobs.get(&view.id).is_none_or(|old| {
                old.state != view.state
                    || old.stage != view.stage
                    || old.attempts != view.attempts
                    || old.progress != view.progress
            });
            if changed {
                self.control.jobs.insert(view.id, view.clone());
                self.control.push(
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
            || cursor.seq > self.control.sequence
            || self.control.events.front().is_some_and(|e| {
                e.seq
                    .parse::<u64>()
                    .is_ok_and(|seq| cursor.seq.saturating_add(1) < seq)
            })
        {
            return Err(EditError::ResyncRequired);
        }
        let events = self
            .control
            .events
            .iter()
            .filter(|e| e.seq.parse::<u64>().is_ok_and(|seq| seq > cursor.seq))
            .map(|e| (**e).clone())
            .collect();
        cursor.seq = self.control.sequence;
        Ok(events)
    }
}
