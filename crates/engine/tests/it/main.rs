//! Integration tests cross the engine folder interfaces.
mod analysis;
mod artifacts;
mod control;
mod providers;
mod runtime;
mod session;
mod store;
mod subtitles;
mod support;
mod transcript;

/// The single wire-semantic helper for tests whose assertion is a decode or
/// limit refusal. It admits the command through the control path so the typed
/// entry points stay free of string dispatch.
pub(crate) mod wire {
    use engine::control::{self, Caller, ControlResult, ErrorCode, Policy, Request, ResponseValue};
    use engine::jobs::{JobRunner, RunnerConfig, SystemClock};
    use engine::{
        ByteBudget, Command, Outcome, Session, control::Error as EngineError, control::Result,
    };
    use serde_json::Value;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    struct Allow;
    impl Policy for Allow {
        fn authorize(&self, _: &Command, _: &project::Project) -> Result<()> {
            Ok(())
        }
    }

    fn runner() -> Result<JobRunner> {
        Ok(JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 4,
                retained: 4,
                network_attempts: 1,
                backoff: std::time::Duration::from_millis(1),
                progress_interval: std::time::Duration::from_millis(10),
            },
            ByteBudget::new(16 << 20)?,
            Arc::new(SystemClock::default()),
        )?)
    }

    /// Run one command by id and JSON through admission, mapping the typed
    /// refusal codes the wire tests assert.
    pub(crate) fn run(s: &mut Session, id: &str, params: Value) -> Result<Outcome> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let request = Request {
            api_version: control::API_VERSION,
            registry_hash: Some(control::registry_hash().into()),
            session_id: s.session_id().into(),
            request_id: format!("wire-{n}"),
            idempotency_key: Some(format!("wire-{n}")),
            base_rev: Some(s.revision()),
            command: id.into(),
            params,
        };
        let mut runner = runner()?;
        let response = s.control_request(
            request,
            Caller {
                principal: "wire",
                external: false,
                policy: &Allow,
                fonts: None,
                sounds: None,
                music: None,
                brand: None,
                media: None,
                chapters: None,
                captions: None,
                vision: None,
            },
            &mut runner,
        )?;
        match response.value {
            ResponseValue::Result(ControlResult::Edit { outcome }) => Ok(outcome),
            ResponseValue::Result(_) => Err(EngineError::Edit(engine::EditError::Invalid(
                "wire helper: non-edit result",
            ))),
            ResponseValue::Error(ErrorCode::UnknownCommand) => Err(EngineError::Control(
                engine::control::ControlError::UnknownCommand,
            )),
            ResponseValue::Error(ErrorCode::InvalidParams) => Err(EngineError::Control(
                engine::control::ControlError::InvalidParams,
            )),
            ResponseValue::Error(ErrorCode::Limit) => {
                Err(EngineError::Control(engine::control::ControlError::Limit))
            }
            ResponseValue::Error(_) => Err(EngineError::Control(
                engine::control::ControlError::InvalidParams,
            )),
        }
    }
}
