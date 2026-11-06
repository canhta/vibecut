//! Retains the bounded receipts and events of one session.
use super::{
    API_VERSION, Event, JobId, JobView, MAX_EVENTS, Payload, Response, Revision, registry_hash,
};
use std::collections::{BTreeMap, VecDeque};

pub(crate) struct Receipt {
    pub(crate) fingerprint: blake3::Hash,
    pub(crate) response: Response,
}
/// Widened to `pub(crate)`: the top control layer admits, replays and resynchronizes
/// through these fields while the kernel only records revision changes.
#[derive(Default)]
pub(crate) struct Ledger {
    pub(crate) receipts: BTreeMap<(String, String), Receipt>,
    pub(crate) receipt_bytes: usize,
    pub(crate) events: VecDeque<std::sync::Arc<Event>>,
    pub(crate) sequence: u64,
    pub(crate) jobs: BTreeMap<JobId, JobView>,
}
/// A cursor holds no queue or project. The bounded engine window is shared by all
/// listeners; gaps are explicit refusals, never a promise of durable replay.
#[derive(Clone, Debug)]
pub struct Cursor {
    pub(crate) session: String,
    pub(crate) seq: u64,
}
pub struct Snapshot {
    pub project: std::sync::Arc<project::Project>,
    pub revision: Revision,
    pub jobs: Vec<JobView>,
    pub cursor: Cursor,
}
impl Ledger {
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
