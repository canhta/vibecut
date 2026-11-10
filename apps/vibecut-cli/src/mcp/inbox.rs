//! Queues validated MCP requests for the engine owner to execute one per poll.
use super::{Grants, Handler};
use crate::args::CliError;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use engine::{
    ByteBudget, EditError, Session,
    control::{self, Caller, ControlResult, MediaAccess, ResponseValue},
    jobs::{JobRunner, RunnerConfig, SystemClock},
    providers::TextAccess,
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

pub(super) const QUEUE: usize = 16;
pub(super) const DEADLINE: Duration = Duration::from_secs(30);

pub(super) struct Pending {
    pub(super) value: Value,
    pub(super) expires: Instant,
    pub(super) reply: oneshot::Sender<Result<Value, EditError>>,
}

/// UI-thread inbox. Polling serializes mutations with ordinary desktop commands.
/// Dropped/cancelled/expired requests are discarded before engine admission.
pub struct Inbox {
    budget: ByteBudget,
    pub(super) projects: std::collections::BTreeMap<project::ProjectId, JobRunner>,
    pub(super) receive: mpsc::Receiver<Pending>,
    runner: JobRunner,
    grants: Grants,
    /// The same caches the CLI control session lends to media generation jobs.
    /// The desktop attaches one instance shared with its timeline displays.
    media: Option<Arc<Mutex<MediaAccess>>>,
    /// The transcript-text adapters and human consent the desktop already owns. An
    /// inbox without them refuses `chapters.generate` and `fillers.classify` with
    /// `consent_required`.
    chapters: Option<Arc<TextAccess>>,
}
impl Inbox {
    pub(super) fn new(grants: Grants, budget: ByteBudget) -> Result<(Handler, Self), CliError> {
        // Without a usable media cache the generation commands refuse; edits
        // still run. Desktop composition attaches its own shared instance.
        let media = crate::session::media_access(budget.clone())
            .ok()
            .map(|access| Arc::new(Mutex::new(access)));
        Self::with_media(grants, budget, media)
    }

    /// Build an inbox with no cache of its own; the caller owns the one
    /// `MediaAccess` both the timeline and the MCP inbox use.
    pub(super) fn with_media(
        grants: Grants,
        budget: ByteBudget,
        media: Option<Arc<Mutex<MediaAccess>>>,
    ) -> Result<(Handler, Self), CliError> {
        let (send, receive) = mpsc::channel(QUEUE);
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 2],
                queue: 32,
                retained: 32,
                network_attempts: 1,
                backoff: Duration::from_millis(100),
                progress_interval: Duration::from_millis(50),
            },
            budget.clone(),
            Arc::new(SystemClock::default()),
        )?;
        Ok((
            Handler::new(send),
            Self {
                budget,
                projects: Default::default(),
                receive,
                runner,
                grants,
                media,
                chapters: None,
            },
        ))
    }

    /// Lend the process media caches the desktop already owns.
    pub fn attach_media(&mut self, media: Arc<Mutex<MediaAccess>>) {
        self.media = Some(media);
    }

    /// Lend the process transcript-text adapters and the human approval they consult.
    /// Without them an agent cannot start a chapter or filler upload.
    pub fn attach_chapters(&mut self, text: Arc<TextAccess>) {
        self.chapters = Some(text);
    }

    /// Process at most one request per tick; jobs continue when clients are idle.
    /// `publish` is the existing composition root's writer-lease save operation.
    pub fn poll(
        &mut self,
        session: &mut Session,
        publish: impl FnOnce(&project::Project, &project::Metadata) -> engine::Result<()>,
    ) -> engine::Result<()> {
        self.runner.tick()?;
        session.observe_jobs(&self.runner);
        let Ok(pending) = self.receive.try_recv() else {
            return Ok(());
        };
        if pending.reply.is_closed() || Instant::now() >= pending.expires {
            return Ok(());
        }
        let result = self.execute(session, pending.value, publish);
        let _ = pending.reply.send(result);
        Ok(())
    }

    /// Desktop routing envelope. `project_id` is a decimal-string ProjectId;
    /// omission selects the focused editor. It is removed before engine admission.
    /// Headless `poll` retains its single-session contract.
    pub fn poll_projects(
        &mut self,
        dispatch: impl FnOnce(Option<project::ProjectId>, &mut Self, Value) -> engine::Result<Value>,
    ) -> engine::Result<()> {
        self.runner.tick()?;
        for runner in self.projects.values_mut() {
            runner.tick()?;
        }
        let Ok(pending) = self.receive.try_recv() else {
            return Ok(());
        };
        if pending.reply.is_closed() || Instant::now() >= pending.expires {
            return Ok(());
        }
        let mut value = pending.value;
        let target = value
            .as_object_mut()
            .and_then(|fields| fields.remove("project_id"))
            .map(serde_json::from_value::<project::ProjectId>)
            .transpose()
            .map_err(|_| EditError::InvalidParams);
        let result = target.and_then(|id| dispatch(id, self, value));
        let _ = pending.reply.send(result);
        Ok(())
    }

    /// Observe jobs for every open project, including an unfocused editor.
    pub fn observe(&self, session: &mut Session) {
        if let Some(runner) = self.projects.get(&session.project().data().id) {
            session.observe_jobs(runner);
        }
    }
    /// Cancel/join only jobs whose project window has closed.
    pub fn retain_projects(&mut self, live: impl Fn(project::ProjectId) -> bool) {
        self.projects.retain(|id, runner| {
            if live(*id) {
                true
            } else {
                runner.shutdown();
                false
            }
        });
    }

    /// Use the desktop project's existing accounting policy for control jobs,
    /// sharing its quota with preview/export rather than creating another one.
    pub fn register_project(
        &mut self,
        id: project::ProjectId,
        budget: ByteBudget,
    ) -> engine::Result<()> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.projects.entry(id) {
            entry.insert(JobRunner::new(
                RunnerConfig {
                    concurrency: [1, 1, 2],
                    queue: 32,
                    retained: 32,
                    network_attempts: 1,
                    backoff: Duration::from_millis(100),
                    progress_interval: Duration::from_millis(50),
                },
                budget,
                Arc::new(SystemClock::default()),
            )?);
        }
        Ok(())
    }

    /// Execute a routed request against its selected session under the same
    /// grants, revision, receipt and job policy as single-session control.
    pub fn execute_project(
        &mut self,
        session: &mut Session,
        value: Value,
        publish: impl FnOnce(&project::Project, &project::Metadata) -> engine::Result<()>,
    ) -> engine::Result<Value> {
        let id = session.project().data().id;
        if !self.projects.contains_key(&id) {
            self.register_project(id, self.budget.child(512 << 20)?)?;
        }
        let Self {
            projects,
            media,
            grants,
            chapters,
            ..
        } = self;
        let runner = projects.get_mut(&id).ok_or(EditError::SessionGone)?;
        session.observe_jobs(runner);
        Self::execute_with(
            runner,
            grants,
            media.as_deref(),
            chapters.as_deref(),
            session,
            value,
            publish,
        )
    }

    fn execute(
        &mut self,
        session: &mut Session,
        value: Value,
        publish: impl FnOnce(&project::Project, &project::Metadata) -> engine::Result<()>,
    ) -> engine::Result<Value> {
        let Self {
            runner,
            media,
            grants,
            chapters,
            ..
        } = self;
        Self::execute_with(
            runner,
            grants,
            media.as_deref(),
            chapters.as_deref(),
            session,
            value,
            publish,
        )
    }
    fn execute_with(
        runner: &mut JobRunner,
        grants: &Grants,
        media: Option<&Mutex<MediaAccess>>,
        chapters: Option<&TextAccess>,
        session: &mut Session,
        value: Value,
        publish: impl FnOnce(&project::Project, &project::Metadata) -> engine::Result<()>,
    ) -> engine::Result<Value> {
        let request = crate::session::build_control_request(session, value)?;
        let caller = Caller {
            principal: "local-mcp",
            external: true,
            policy: grants,
            fonts: None,
            media,
            chapters,
        };
        let response = session.control_routed(request, caller, runner, publish)?;
        let mut value = serde_json::to_value(&response).map_err(|_| EditError::InvalidParams)?;
        // Send JPEG as MCP image content, not an array of byte-valued JSON numbers.
        if let ResponseValue::Result(ControlResult::Job { snapshot }) = &response.value
            && let Some(result) = &snapshot.result
        {
            let frames = serde_json::to_value(result).map_err(|_| EditError::InvalidParams)?;
            if let Some(frames) = frames.get("frames").and_then(Value::as_array) {
                let mut images = Vec::new();
                for frame in frames {
                    let bytes: Vec<u8> =
                        serde_json::from_value(frame.get("jpeg").cloned().unwrap_or(Value::Null))
                            .map_err(|_| EditError::InvalidParams)?;
                    images.push(json!({"data":STANDARD.encode(bytes),"mimeType":"image/jpeg"}));
                }
                value
                    .as_object_mut()
                    .ok_or(EditError::InvalidParams)?
                    .insert("images".into(), json!(images));
                if let Some(frames) = value
                    .pointer_mut("/value/value/snapshot/result/frames")
                    .and_then(Value::as_array_mut)
                {
                    for frame in frames {
                        if let Some(fields) = frame.as_object_mut() {
                            fields.remove("jpeg");
                        }
                    }
                }
            }
        }
        control::bounded_json(&value, control::MAX_RESPONSE_BYTES)?;
        Ok(value)
    }
}
impl Drop for Inbox {
    fn drop(&mut self) {
        self.runner.shutdown();
        for runner in self.projects.values_mut() {
            runner.shutdown();
        }
    }
}
