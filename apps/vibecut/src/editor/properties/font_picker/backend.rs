//! Adapts the picker to the registered engine `fonts.*` commands.
use engine::control::{
    API_VERSION, Caller, ControlResult, FontAccess, FontEntry, FontLoad, JobView, Policy, Request,
    ResponseValue, registry_hash,
};
use engine::jobs::{JobId, JobRunner, RunnerConfig, SystemClock};
use engine::{ByteBudget, EditError, Session};
use std::sync::Arc;
use std::time::Duration;

/// The three catalog operations the picker consumes plus job progress and
/// cancellation. A second adapter is the test host; production uses
/// [`EngineBackend`].
pub trait Backend: Send {
    fn list(&mut self, session: &mut Session, query: Option<&str>)
    -> crate::Result<Vec<FontEntry>>;
    fn load(&mut self, session: &mut Session, family: &str) -> crate::Result<FontLoad>;
    fn download(&mut self, session: &mut Session, family: &str) -> crate::Result<JobId>;
    fn job(&mut self, job: JobId) -> Option<JobView>;
    fn cancel(&mut self, session: &mut Session, job: JobId) -> crate::Result<()>;
}

/// Production adapter: the shared engine font store behind the registered
/// `fonts.list`, `fonts.load`, `fonts.download` and `job_cancel` commands.
pub struct EngineBackend {
    store: Arc<libraries::fonts::FontStore>,
    runner: JobRunner,
    sequence: u64,
}
impl EngineBackend {
    pub fn native() -> crate::Result<Self> {
        let store = Arc::new(libraries::fonts::FontStore::native()?);
        Self::with_store(store)
    }
    pub fn with_store(store: Arc<libraries::fonts::FontStore>) -> crate::Result<Self> {
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 1,
                retained: 4,
                network_attempts: 1,
                backoff: Duration::from_millis(250),
                progress_interval: Duration::from_millis(40),
            },
            ByteBudget::new(64 << 20)?,
            Arc::new(SystemClock::default()),
        )?;
        Ok(Self {
            store,
            runner,
            sequence: 0,
        })
    }
    fn request(
        &mut self,
        session: &mut Session,
        command: &str,
        params: serde_json::Value,
    ) -> crate::Result<ControlResult> {
        let store = &self.store;
        let runner = &mut self.runner;
        let list = |query: Option<&str>| store.list(query);
        let load = |family: &str| store.load(family);
        let download = |runner: &mut JobRunner, family: &str, accepted: bool| {
            store
                .submit_download(runner, family, accepted)
                .map(|(id, _)| id)
        };
        let access = FontAccess::new(&list, &load, &download);
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        let request_id = format!("vibecut-font-{}-{sequence}", std::process::id());
        let request = Request {
            api_version: API_VERSION,
            registry_hash: Some(registry_hash().into()),
            session_id: session.session_id().into(),
            idempotency_key: Some(request_id.clone()),
            request_id,
            base_rev: Some(session.revision()),
            command: command.into(),
            params,
        };
        let response = session.control_request(
            request,
            Caller {
                principal: "vibecut-desktop",
                external: false,
                policy: &Allow,
                fonts: Some(&access),
                media: None,
            },
            runner,
        )?;
        match response.value {
            ResponseValue::Result(value) => Ok(value),
            ResponseValue::Error(code) => Err(EditError::Invalid(match code {
                engine::control::ErrorCode::Permission => "font command refused",
                engine::control::ErrorCode::QueueFull => "font queue full",
                engine::control::ErrorCode::Cancelled => "font command cancelled",
                _ => "font command failed",
            })
            .into()),
        }
    }
}
impl Backend for EngineBackend {
    fn list(
        &mut self,
        session: &mut Session,
        query: Option<&str>,
    ) -> crate::Result<Vec<FontEntry>> {
        let params = match query {
            Some(query) => serde_json::json!({"query": query}),
            None => serde_json::json!({}),
        };
        match self.request(session, "fonts.list", params)? {
            ControlResult::Fonts { entries } => Ok(entries),
            _ => Err(crate::Error::Invalid("fonts.list result")),
        }
    }
    fn load(&mut self, session: &mut Session, family: &str) -> crate::Result<FontLoad> {
        match self.request(session, "fonts.load", serde_json::json!({"family": family}))? {
            ControlResult::Font { font } => Ok(font),
            _ => Err(crate::Error::Invalid("fonts.load result")),
        }
    }
    fn download(&mut self, session: &mut Session, family: &str) -> crate::Result<JobId> {
        match self.request(
            session,
            "fonts.download",
            serde_json::json!({"family": family, "accept_license": true}),
        )? {
            ControlResult::JobAccepted { job, .. } => Ok(job),
            _ => Err(crate::Error::Invalid("fonts.download result")),
        }
    }
    fn job(&mut self, job: JobId) -> Option<JobView> {
        self.runner.snapshot(job).map(JobView::from)
    }
    fn cancel(&mut self, session: &mut Session, job: JobId) -> crate::Result<()> {
        match self.request(session, "job_cancel", serde_json::json!({"job": job}))? {
            ControlResult::CancelRequested { .. } => Ok(()),
            _ => Err(crate::Error::Invalid("job_cancel result")),
        }
    }
}
struct Allow;
impl Policy for Allow {
    fn authorize(&self, _: &engine::Command, _: &project::Project) -> engine::Result<()> {
        Ok(())
    }
}
