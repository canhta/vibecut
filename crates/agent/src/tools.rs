//! Generates the model's tools from the registry and runs each call against the fork.
use crate::cancel::CancelFlag;
use crate::event::{AgentEvent, EndReason};
use crate::policy::{AgentPolicy, Reason};
use crate::reads;
use ai::text::conversation::{DataKind, ImageType, ToolImage, ToolSpec};
use engine::control::{
    API_VERSION, Caller, ControlResult, ErrorCode, Failure, JobOutput, JobView, Request,
    ResponseValue, registry_hash,
};
use engine::jobs::{JobId, JobRunner, JobState};
use engine::{Effect, Fork, Session, registry};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// The principal every agent request carries.
pub const PRINCIPAL: &str = "vibecut-agent";

const PROJECT_SUMMARY: &str = "project_summary";
const TIMELINE_READ: &str = "timeline_read";
const TRANSCRIPT_READ: &str = "transcript_read";
const COMMAND_LIST: &str = "command_list";
const COMMAND_RUN: &str = "command_run";
const JOB_STATUS: &str = "job_status";
const PREVIEW_FRAME: &str = "preview_frame";

/// Bytes one tool result may carry; a larger one is refused so history stays inside the
/// conversation's context ceiling.
const MAX_RESULT_BYTES: usize = 32 * 1024;
const MAX_PAGE: u64 = 50;
const MAX_COMMANDS_PAGE: u64 = 8;
const MAX_WAIT_MS: u64 = 30_000;
/// Bytes of JPEG one frame may carry into the conversation; the engine renders at most 640
/// pixels on the long edge, and a larger encoding is refused so one frame stays a small part
/// of the context ceiling.
const MAX_FRAME_BYTES: usize = 160 * 1024;
const MAX_FRAME_NUMBER: u64 = u32::MAX as u64;
/// How long one `preview_frame` call waits for its render.
const FRAME_WAIT: Duration = Duration::from_millis(MAX_WAIT_MS);
const POLL: Duration = Duration::from_millis(20);
/// How long ending a turn waits for cancelled jobs to settle.
const DRAIN: Duration = Duration::from_secs(2);

/// The tools the model is offered, schemas generated from the registry. `preview_frame` is
/// offered only when the conversation shares frames.
pub fn tool_specs(frames: bool) -> Vec<ToolSpec> {
    let policy = AgentPolicy::new(frames);
    let allowed: Vec<&str> = registry()
        .iter()
        .map(|spec| spec.id)
        .filter(|id| policy.check_command(id).is_ok())
        .collect();
    let page = |max: u64| {
        json!({"type": "object", "additionalProperties": false, "properties": {
            "cursor": {"type": "integer", "minimum": 0},
            "limit": {"type": "integer", "minimum": 1, "maximum": max},
        }})
    };
    let mut timeline = page(MAX_PAGE);
    if let Some(properties) = timeline
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        properties.insert("track".into(), json!({"type": "integer", "minimum": 0}));
    }
    let mut list = page(MAX_COMMANDS_PAGE);
    if let Some(properties) = list.get_mut("properties").and_then(Value::as_object_mut) {
        properties.insert("prefix".into(), json!({"type": "string", "maxLength": 64}));
    }
    let spec = |name: &str, description: &str, input_schema: Value| ToolSpec {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema,
    };
    let mut specs = vec![
        spec(
            PROJECT_SUMMARY,
            "Project name, canvas, duration, tracks and media of the working copy.",
            json!({"type": "object", "additionalProperties": false, "properties": {}}),
        ),
        spec(
            TIMELINE_READ,
            "One page of timeline items, top track first, with ids, kinds, start and end ticks.",
            timeline,
        ),
        spec(
            TRANSCRIPT_READ,
            "One page of caption cues in output time with their text.",
            page(MAX_PAGE),
        ),
        spec(
            COMMAND_LIST,
            "Edit commands you may run, with the JSON Schema of each one's params. Filter by id prefix such as timeline.",
            list,
        ),
        spec(
            COMMAND_RUN,
            "Run one edit command on the working copy; use command batch with params {commands: [{command, params}]} to run several as one step. The result is the real outcome. Nothing reaches the user's project until they apply the changes.",
            json!({"type": "object", "additionalProperties": false,
            "required": ["command", "params"],
            "properties": {
                "command": {"type": "string", "enum": allowed},
                "params": {"type": "object"},
            }}),
        ),
        spec(
            JOB_STATUS,
            "State and progress of a job a command started; wait_ms waits for it to finish.",
            json!({"type": "object", "additionalProperties": false, "required": ["job"],
            "properties": {
                "job": {"type": "integer", "minimum": 0},
                "wait_ms": {"type": "integer", "minimum": 0, "maximum": MAX_WAIT_MS},
            }}),
        ),
    ];
    if frames {
        specs.push(spec(
            PREVIEW_FRAME,
            "Render one frame of the working copy and see it as an image, at most 640 pixels on the long edge. frame is a frame number at the project's frame rate. One frame per call.",
            json!({"type": "object", "additionalProperties": false, "required": ["frame"],
            "properties": {
                "frame": {"type": "integer", "minimum": 0, "maximum": MAX_FRAME_NUMBER},
            }}),
        ));
    }
    specs
}

/// What one tool call produced for the model.
pub(crate) struct ToolOutput {
    pub(crate) content: String,
    pub(crate) is_error: bool,
    pub(crate) kinds: BTreeSet<DataKind>,
    /// Frames the model sees after `content`.
    pub(crate) images: Vec<ToolImage>,
    /// The turn must end now for this reason.
    pub(crate) stop: Option<EndReason>,
}
impl ToolOutput {
    fn ok(value: &Value, kinds: &[DataKind]) -> Self {
        match serde_json::to_string(value) {
            Ok(content) if content.len() <= MAX_RESULT_BYTES => Self {
                content,
                is_error: false,
                kinds: kinds.iter().copied().collect(),
                images: Vec::new(),
                stop: None,
            },
            _ => Self::error("too_large", "The result is too large; ask for less."),
        }
    }
    fn error(code: &str, detail: &str) -> Self {
        Self {
            content: json!({"error": code, "detail": detail}).to_string(),
            is_error: true,
            kinds: BTreeSet::new(),
            images: Vec::new(),
            stop: None,
        }
    }
    fn refused(command: &str, reason: Reason) -> Self {
        Self {
            content: json!({"error": "refused", "command": command, "reason": reason, "detail": reason.text()})
                .to_string(),
            is_error: true,
            kinds: BTreeSet::new(),
            images: Vec::new(),
            stop: None,
        }
    }
}

fn fields<'a>(input: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>, ToolOutput> {
    let map = input
        .as_object()
        .ok_or_else(|| ToolOutput::error("invalid_input", "The input must be an object."))?;
    if map.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ToolOutput::error(
            "invalid_input",
            "The input has a field the tool does not define.",
        ));
    }
    Ok(map)
}

fn number(map: &Map<String, Value>, key: &str, default: u64, max: u64) -> Result<u64, ToolOutput> {
    match map.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|number| *number <= max)
            .ok_or_else(|| {
                ToolOutput::error(
                    "invalid_input",
                    "A number is missing, negative or too large.",
                )
            }),
    }
}

fn to_usize(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// The fork, its runner and the bookkeeping of one turn's tool calls.
pub(crate) struct Toolbox<'r> {
    fork: Fork,
    runner: &'r mut JobRunner,
    policy: AgentPolicy,
    cancel: CancelFlag,
    allowed: BTreeSet<DataKind>,
    sequence: u64,
    jobs: Vec<JobId>,
    refusals: usize,
}
impl<'r> Toolbox<'r> {
    pub(crate) fn new(
        fork: Fork,
        runner: &'r mut JobRunner,
        cancel: CancelFlag,
        allowed: BTreeSet<DataKind>,
    ) -> Self {
        Self {
            fork,
            runner,
            policy: AgentPolicy::new(allowed.contains(&DataKind::Frames)),
            cancel,
            allowed,
            sequence: 0,
            jobs: Vec::new(),
            refusals: 0,
        }
    }

    pub(crate) fn fork(&self) -> &Fork {
        &self.fork
    }

    /// Commands the policy refused so far, nested batch children included.
    pub(crate) fn refusals(&self) -> usize {
        self.refusals
    }

    pub(crate) fn into_fork(self) -> Fork {
        self.fork
    }

    /// Run one call. Every refusal is a result the model reads; none reaches an effect.
    pub(crate) fn run(
        &mut self,
        name: &str,
        input: &Value,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> ToolOutput {
        let result = match name {
            PROJECT_SUMMARY => self.project_summary(input),
            TIMELINE_READ => self.timeline_read(input),
            TRANSCRIPT_READ => self.transcript_read(input),
            COMMAND_LIST => Self::command_list(input, &self.policy),
            COMMAND_RUN => self.command_run(input),
            JOB_STATUS => self.job_status(input, emit),
            PREVIEW_FRAME => self.preview_frame(input, emit),
            _ => Err(ToolOutput::error("unknown_tool", "No such tool.")),
        };
        result.unwrap_or_else(|output| output)
    }

    /// Ask every job this turn started or watched to stop, then wait a bounded time for
    /// them to settle so none outlives the turn.
    pub(crate) fn cancel_jobs(&mut self) {
        for job in &self.jobs {
            let _ = self.runner.cancel_job(*job);
        }
        let until = Instant::now() + DRAIN;
        while Instant::now() < until {
            if self.runner.tick().is_err() {
                return;
            }
            let running = self.jobs.iter().any(|job| {
                self.runner
                    .snapshot(*job)
                    .is_some_and(|snapshot| !terminal(snapshot.state))
            });
            if !running {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn project_summary(&self, input: &Value) -> Result<ToolOutput, ToolOutput> {
        fields(input, &[])?;
        let names = self.allowed.contains(&DataKind::ClipNames);
        let value = reads::project_summary(self.fork.session(), names);
        let mut kinds = vec![DataKind::ProjectSummary];
        if names {
            kinds.push(DataKind::ClipNames);
        }
        Ok(ToolOutput::ok(&value, &kinds))
    }

    fn timeline_read(&self, input: &Value) -> Result<ToolOutput, ToolOutput> {
        let map = fields(input, &["track", "cursor", "limit"])?;
        let track = match map.get("track") {
            None => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .ok_or_else(|| ToolOutput::error("invalid_input", "track must be an id."))?,
            ),
        };
        let cursor = number(map, "cursor", 0, u64::MAX)?;
        let limit = number(map, "limit", 20, MAX_PAGE)?.max(1);
        let value = reads::timeline_read(
            self.fork.session(),
            track,
            to_usize(cursor),
            to_usize(limit),
        );
        Ok(ToolOutput::ok(&value, &[DataKind::ProjectSummary]))
    }

    fn transcript_read(&self, input: &Value) -> Result<ToolOutput, ToolOutput> {
        let map = fields(input, &["cursor", "limit"])?;
        if !self.allowed.contains(&DataKind::TranscriptText) {
            return Err(ToolOutput::error(
                "not_shared",
                "Transcript text is not shared in this conversation.",
            ));
        }
        let cursor = number(map, "cursor", 0, u64::MAX)?;
        let limit = number(map, "limit", 20, MAX_PAGE)?.max(1);
        let value = reads::transcript_read(self.fork.session(), to_usize(cursor), to_usize(limit))
            .map_err(|detail| ToolOutput::error("unreadable", &detail))?;
        Ok(ToolOutput::ok(&value, &[DataKind::TranscriptText]))
    }

    fn command_list(input: &Value, policy: &AgentPolicy) -> Result<ToolOutput, ToolOutput> {
        let map = fields(input, &["cursor", "limit", "prefix"])?;
        let cursor = to_usize(number(map, "cursor", 0, u64::MAX)?);
        let limit = to_usize(number(map, "limit", MAX_COMMANDS_PAGE, MAX_COMMANDS_PAGE)?.max(1));
        let prefix = match map.get("prefix") {
            None => "",
            Some(value) => value
                .as_str()
                .ok_or_else(|| ToolOutput::error("invalid_input", "prefix must be text."))?,
        };
        let mut matching = registry()
            .iter()
            .filter(|spec| spec.id.starts_with(prefix) && policy.check_command(spec.id).is_ok())
            .skip(cursor)
            .peekable();
        let mut entries: Vec<Value> = Vec::new();
        let mut bytes = 0usize;
        while entries.len() < limit {
            let Some(spec) = matching.peek() else { break };
            let entry = json!({
                "id": spec.id,
                "label": spec.label,
                "effect": spec.effect,
                "params": spec.params_schema,
            });
            let size = entry.to_string().len();
            if bytes + size > MAX_RESULT_BYTES - 1024 && !entries.is_empty() {
                break;
            }
            bytes += size;
            entries.push(entry);
            matching.next();
        }
        let next = matching.peek().is_some().then(|| cursor + entries.len());
        Ok(ToolOutput::ok(
            &json!({"commands": entries, "next_cursor": next}),
            &[],
        ))
    }

    /// The first refused command in a request, searching a batch's children too.
    fn refusal(&self, id: &str, params: &Value, depth: usize) -> Option<(String, Reason)> {
        if let Err(reason) = self.policy.check_command(id) {
            return Some((id.to_owned(), reason));
        }
        if depth >= 8 {
            return None;
        }
        let children = params.get("commands").and_then(Value::as_array)?;
        let none = Value::Null;
        children.iter().find_map(|child| {
            let id = child.get("command").and_then(Value::as_str)?;
            self.refusal(id, child.get("params").unwrap_or(&none), depth + 1)
        })
    }

    fn command_run(&mut self, input: &Value) -> Result<ToolOutput, ToolOutput> {
        let map = fields(input, &["command", "params"])?;
        let command = map
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolOutput::error("invalid_input", "command must be an id."))?;
        let params = map.get("params").cloned().unwrap_or_else(|| json!({}));
        if let Some((id, reason)) = self.refusal(command, &params, 0) {
            self.refusals += 1;
            return Err(ToolOutput::refused(&id, reason));
        }
        let effect = registry()
            .iter()
            .find(|spec| spec.id == command)
            .map(|spec| spec.effect)
            .ok_or_else(|| ToolOutput::error("unknown_command", "No such command."))?;
        let response = self.send(command, params, effect != Effect::Read);
        let value = match response {
            Err(code) => return Err(ToolOutput::error(code_name(code), "The command failed.")),
            Ok(ResponseValue::Error(ErrorCode::Permission)) => {
                let mut output = ToolOutput::error(
                    "consent_required",
                    "The command needs consent this conversation does not grant. The user must approve it.",
                );
                output.stop = Some(EndReason::ConsentRequired);
                return Err(output);
            }
            Ok(ResponseValue::Error(code)) => {
                return Err(ToolOutput::error(code_name(code), "The command failed."));
            }
            Ok(ResponseValue::Result(result)) => result,
        };
        if let ControlResult::JobAccepted { job, .. } = &value {
            self.watch(*job);
        }
        let encoded = serde_json::to_value(&value)
            .map_err(|_| ToolOutput::error("unreadable", "The result cannot be encoded."))?;
        Ok(ToolOutput::ok(&encoded, &[DataKind::ProjectSummary]))
    }

    fn job_status(
        &mut self,
        input: &Value,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<ToolOutput, ToolOutput> {
        let map = fields(input, &["job", "wait_ms"])?;
        let job = map
            .get("job")
            .and_then(Value::as_u64)
            .ok_or_else(|| ToolOutput::error("invalid_input", "job must be an id."))?;
        let wait = Duration::from_millis(number(map, "wait_ms", 0, MAX_WAIT_MS)?);
        let view = self.wait_job(job, wait, emit)?;
        let value = json!({
            "job": job,
            "state": view.state,
            "progress": view.progress,
            "error": view.error,
            "has_output": view.output.is_some(),
        });
        Ok(ToolOutput::ok(&value, &[DataKind::ProjectSummary]))
    }

    /// Render one frame of the fork and hand it to the model as an image. The frame is
    /// requested through the same `preview` command an outside agent runs, so admission,
    /// limits and the fork's current state are the engine's.
    fn preview_frame(
        &mut self,
        input: &Value,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<ToolOutput, ToolOutput> {
        let map = fields(input, &["frame"])?;
        if !self.allowed.contains(&DataKind::Frames) {
            return Err(ToolOutput::error(
                "not_shared",
                "Frames are not shared in this conversation.",
            ));
        }
        if !map.contains_key("frame") {
            return Err(ToolOutput::error("invalid_input", "frame is required."));
        }
        let frame = number(map, "frame", 0, MAX_FRAME_NUMBER)?;
        let job = match self.send("preview", json!({"frames": [frame]}), true) {
            Ok(ResponseValue::Result(ControlResult::JobAccepted { job, .. })) => job,
            Ok(ResponseValue::Error(code)) | Err(code) => {
                return Err(ToolOutput::error(
                    code_name(code),
                    "The frame could not be rendered.",
                ));
            }
            Ok(ResponseValue::Result(_)) => {
                return Err(ToolOutput::error("unreadable", "Not a job."));
            }
        };
        self.watch(job);
        let view = self.wait_job(job.0, FRAME_WAIT, emit)?;
        let Some(JobOutput::Preview(result)) = view.output else {
            return Err(ToolOutput::error(
                "preview_failed",
                "The frame could not be rendered.",
            ));
        };
        let rendered = result
            .frames
            .first()
            .ok_or_else(|| ToolOutput::error("preview_failed", "No frame was rendered."))?;
        if rendered.jpeg.len() > MAX_FRAME_BYTES {
            return Err(ToolOutput::error(
                "too_large",
                "The frame is too large to show.",
            ));
        }
        let clips: Vec<u64> = rendered.clips.iter().map(|id| id.0).collect();
        let value = json!({
            "frame": rendered.frame,
            "width": rendered.width,
            "height": rendered.height,
            "clips": clips,
        });
        let mut output = ToolOutput::ok(&value, &[DataKind::ProjectSummary, DataKind::Frames]);
        if !output.is_error {
            output
                .images
                .push(ToolImage::new(ImageType::Jpeg, &rendered.jpeg));
        }
        Ok(output)
    }

    /// Poll a job until it ends, the wait runs out or the user cancels, reporting progress.
    fn wait_job(
        &mut self,
        job: u64,
        wait: Duration,
        emit: &mut dyn FnMut(AgentEvent),
    ) -> Result<JobView, ToolOutput> {
        let id = JobId(job);
        self.watch(id);
        let deadline = Instant::now() + wait;
        let mut last: Option<(JobState, u8)> = None;
        loop {
            self.runner
                .tick()
                .map_err(|_| ToolOutput::error("job_failed", "The job runner failed."))?;
            let view = match self.send("job_status", json!({"job": job}), false) {
                Ok(ResponseValue::Result(ControlResult::Job { snapshot })) => snapshot,
                Ok(ResponseValue::Error(code)) | Err(code) => {
                    return Err(ToolOutput::error(code_name(code), "The job is unknown."));
                }
                Ok(ResponseValue::Result(_)) => {
                    return Err(ToolOutput::error("unreadable", "Not a job."));
                }
            };
            let percent = (view.progress.clamp(0.0, 1.0) * 100.0).round() as u8;
            if last != Some((view.state, percent)) {
                last = Some((view.state, percent));
                emit(AgentEvent::JobProgress {
                    job,
                    state: state_name(view.state).to_owned(),
                    percent,
                });
            }
            if terminal(view.state) || Instant::now() >= deadline || self.cancel.is_cancelled() {
                return Ok(view);
            }
            std::thread::sleep(POLL);
        }
    }

    fn watch(&mut self, job: JobId) {
        if !self.jobs.contains(&job) {
            self.jobs.push(job);
        }
    }

    /// One request on the fork through the control path an outside agent uses.
    fn send(
        &mut self,
        command: &str,
        params: Value,
        effectful: bool,
    ) -> Result<ResponseValue, ErrorCode> {
        self.sequence += 1;
        let key = format!("agent-{}", self.sequence);
        let session: &Session = self.fork.session();
        let request = Request {
            api_version: API_VERSION,
            registry_hash: Some(registry_hash().into()),
            session_id: session.session_id().into(),
            request_id: key.clone(),
            idempotency_key: effectful.then_some(key),
            base_rev: effectful.then(|| session.revision()),
            command: command.into(),
            params,
        };
        let caller = Caller {
            principal: PRINCIPAL,
            external: true,
            policy: &self.policy,
            fonts: None,
            sounds: None,
            music: None,
            stickers: None,
            brand: None,
            media: None,
            chapters: None,
            captions: None,
            vision: None,
        };
        self.fork
            .control_request(request, caller, &mut *self.runner)
            .map(|response| response.value)
            .map_err(|error| error.code())
    }
}

fn terminal(state: JobState) -> bool {
    matches!(
        state,
        JobState::Completed | JobState::Failed | JobState::Cancelled
    )
}

fn state_name(state: JobState) -> &'static str {
    match state {
        JobState::Queued => "queued",
        JobState::Running => "running",
        JobState::Cancelling => "cancelling",
        JobState::Publishing => "publishing",
        JobState::Backoff => "backoff",
        JobState::Completed => "completed",
        JobState::Cancelled => "cancelled",
        JobState::Failed => "failed",
    }
}

fn code_name(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::UnknownCommand => "unknown_command",
        ErrorCode::InvalidParams => "invalid_params",
        ErrorCode::Limit => "limit",
        ErrorCode::SchemaMismatch => "schema_mismatch",
        ErrorCode::StaleRevision => "stale_revision",
        ErrorCode::Permission => "permission",
        ErrorCode::JobNotRetained => "job_not_retained",
        ErrorCode::Cancelled => "cancelled",
        ErrorCode::QueueFull => "queue_full",
        ErrorCode::TransactionActive => "transaction_active",
        ErrorCode::Refused => "refused",
        ErrorCode::Unavailable => "unavailable",
        ErrorCode::Conflict => "conflict",
        ErrorCode::StaleResult => "stale_result",
        ErrorCode::Auth => "auth",
        ErrorCode::Quota => "quota",
        ErrorCode::PlanRequired => "plan_required",
        ErrorCode::RateLimit => "rate_limit",
        ErrorCode::Network => "network",
        _ => "failed",
    }
}
