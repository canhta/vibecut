//! Admits transport-neutral commands and runs control-only commands, the top engine layer.
use crate::{Command, Result};

pub use crate::session::wire::{
    API_VERSION, ControlResult, Cursor, ErrorCode, Event, JobView, LIMITS, Limits,
    MAX_BATCH_COMMANDS, MAX_BATCH_DEPTH, MAX_COLLECTION, MAX_DEPTH, MAX_EVENTS, MAX_FRAME_BYTES,
    MAX_KEY_BYTES, MAX_NODES, MAX_PATH_BYTES, MAX_PREVIEW_CLIPS, MAX_PREVIEW_EDGE,
    MAX_PREVIEW_FRAMES, MAX_RECEIPT_BYTES, MAX_RECEIPTS, MAX_RESPONSE_BYTES, MAX_SAFE_INTEGER,
    MAX_TEXT_BYTES, PREVIEW_QUALITY, Payload, PreviewFrame, PreviewResult, ProviderView,
    RECEIPT_RESERVATION, Request, Response, ResponseValue, Revision, Snapshot, bounded_json,
    contract_schema, parse_frame, reference_markdown, registry_hash,
};

mod admission;
mod dispatch;
mod receipts;
mod resync;

/// Supplied by the trusted composition root, never deserialized from client JSON.
/// Implementations admit source reads, destination writes/overwrite and provider
/// consent through their owners. The engine calls this on every child before IO.
pub trait Policy {
    /// Trusted snapshot replacement grant; the request itself has no overwrite flag.
    /// Without a grant, staging and final publication both use atomic no-clobber.
    fn snapshot_overwrite(&self) -> bool {
        false
    }

    fn authorize(&self, command: &Command, project: &project::Project) -> Result<()>;
}
pub struct Caller<'a> {
    pub principal: &'a str,
    pub external: bool,
    pub policy: &'a dyn Policy,
    /// The process font store that serves `fonts.list`, `fonts.load` and `fonts.download`.
    /// It is composition state, not project state; without one those commands are refused
    /// with [`crate::fonts::FontError::Unavailable`].
    pub fonts: Option<&'a crate::fonts::FontStore>,
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        reason = "injected capacity and generation exhaustion"
    )]
    use super::*;
    use crate::{EditError, Session, jobs::JobRunner};
    use serde_json::json;
    struct Allow;
    impl Policy for Allow {
        fn authorize(&self, _: &Command, _: &project::Project) -> Result<()> {
            Ok(())
        }
    }
    #[test]
    fn byte_capacity_is_reserved_before_mutation_and_generation_exhaustion_is_atomic() {
        let mut session = Session::new(
            project::Project::empty(project::ProjectId(0), "before".into()).unwrap(),
            10,
        )
        .unwrap();
        let mut runner = JobRunner::new(
            crate::jobs::RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 1,
                retained: 1,
                network_attempts: 1,
                backoff: std::time::Duration::from_millis(1),
                progress_interval: std::time::Duration::from_millis(10),
            },
            crate::ByteBudget::new(1 << 20).unwrap(),
            std::sync::Arc::new(crate::jobs::SystemClock::default()),
        )
        .unwrap();
        let before = session.project().clone();
        session.ledger.receipt_bytes = MAX_RECEIPT_BYTES - RECEIPT_RESERVATION + 1;
        let request = Request {
            api_version: API_VERSION,
            registry_hash: Some(registry_hash().into()),
            session_id: session.session_id().into(),
            request_id: "capacity".into(),
            idempotency_key: Some("new".into()),
            base_rev: Some(session.revision()),
            command: "project.rename".into(),
            params: json!({"name":"never"}),
        };
        let response = session
            .control_request(
                request,
                Caller {
                    principal: "test",
                    external: true,
                    policy: &Allow,
                    fonts: None,
                },
                &mut runner,
            )
            .unwrap();
        assert!(matches!(
            response.value,
            ResponseValue::Error(ErrorCode::ReceiptCapacity)
        ));
        assert_eq!(session.project(), &before);
        assert_eq!(session.history_len(), (0, 0));
        session.generation = u64::MAX;
        assert!(matches!(
            session.apply(Command::RenameProject {
                name: "never".into()
            }),
            Err(EditError::Limit)
        ));
        assert_eq!(session.project(), &before);
        session.generation = u64::MAX - 2;
        let result = session.transaction(|s| {
            s.apply(Command::RenameProject {
                name: "preview".into(),
            })?;
            s.apply(Command::RenameProject {
                name: "never".into(),
            })
        });
        assert!(matches!(result, Err(EditError::Limit)));
        assert_eq!(session.project(), &before);
        assert_eq!(session.history_len(), (0, 0));
        assert_eq!(session.generation, u64::MAX);
    }
}
