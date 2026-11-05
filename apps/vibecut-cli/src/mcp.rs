//! MCP composition. Engine admission remains the sole command/receipt owner.
mod socket;
#[cfg(test)]
mod tests;
mod transport;

use crate::{CliError, Output};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use engine::{
    ByteBudget, Command, EditError, Session,
    control::{self, Caller, ControlResult, Policy, ResponseValue},
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use rmcp::{
    ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
        Implementation, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
        ServerCapabilities, ServerConfig, Tool,
    },
    service::{RequestContext, RoleServer},
};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

const QUEUE: usize = 16;
const DEADLINE: Duration = Duration::from_secs(30);
const CURATED: &[&str] = &[
    "inspect",
    "preview",
    "undo",
    "redo",
    "export",
    "job_status",
    "job_cancel",
    "batch",
];

/// Trusted command-line grants. A client request can never broaden them.
#[derive(Clone, Default)]
pub struct Grants {
    output: Option<PathBuf>,
    selected_output: Option<PathBuf>,
}
impl Grants {
    /// Admit a pre-existing output directory. Overwrites and imported paths remain refused.
    pub fn output(directory: &Path) -> Result<Self, CliError> {
        let output = std::fs::canonicalize(directory)?;
        if !output.is_dir() {
            return Err(CliError::Rejected("output directory is not a directory"));
        }
        Ok(Self {
            output: Some(output),
            selected_output: Some(if directory.is_absolute() {
                directory.to_owned()
            } else {
                std::env::current_dir()?.join(directory)
            }),
        })
    }

    fn output_path(
        &self,
        path: &Path,
        overwrite: bool,
        project: &project::Project,
    ) -> engine::Result<()> {
        let root = self.output.as_ref().ok_or(EditError::Permission)?;
        if overwrite || !path.is_absolute() || path.file_name().is_none() {
            return Err(EditError::Permission);
        }
        let parent = path.parent().ok_or(EditError::Permission)?;
        // Only resolve a directory explicitly selected by the human.
        // Resolving an arbitrary client parent first could touch an
        // ungranted filesystem or initiate a Windows UNC connection.
        if parent != root && Some(parent) != self.selected_output.as_deref() {
            return Err(EditError::Permission);
        }
        let parent = std::fs::canonicalize(parent).map_err(|_| EditError::Permission)?;
        if &parent != root {
            return Err(EditError::Permission);
        }
        // An output must not replace even a currently missing source path.
        if project
            .data()
            .assets
            .iter()
            .any(|asset| asset.path() == path)
        {
            return Err(EditError::Permission);
        }
        Ok(())
    }
}
impl Policy for Grants {
    fn authorize(&self, command: &Command, project: &project::Project) -> engine::Result<()> {
        match command {
            // Source paths are capabilities established by the human opening the project.
            // Caller-provided SourceAsset would otherwise become a file-read primitive.
            Command::AddAsset { .. }
            | Command::CreateProject { .. }
            | Command::DuplicateProject { .. }
            | Command::DeleteProject {}
            | Command::ThumbnailProject {}
            | Command::BeginGesture {}
            | Command::CommitGesture {}
            | Command::CancelGesture {} => Err(EditError::Permission),
            Command::Export {
                path, overwrite, ..
            }
            | Command::ExportSubtitles {
                path, overwrite, ..
            }
            | Command::BurnSubtitles {
                path, overwrite, ..
            } => self.output_path(path, *overwrite, project),
            Command::Snapshot { path, .. } => self.output_path(path, false, project),
            // Each command is reviewed, even within an existing effect class.
            Command::CreateScene { .. }
            | Command::DeleteScenes { .. }
            | Command::SwitchScene { .. }
            | Command::RenameScene { .. }
            | Command::RenameProject { .. }
            | Command::SaveProject { .. }
            | Command::SetMaster { .. }
            | Command::SetViewState { .. }
            | Command::RemoveAssets { .. }
            | Command::AddTrack { .. }
            | Command::RemoveTrack { .. }
            | Command::SetTrackMuted { .. }
            | Command::SetTrackHidden { .. }
            | Command::SetTrackMagnet { .. }
            | Command::SetTrackFlags { .. }
            | Command::RenameTrack { .. }
            | Command::ReorderTrack { .. }
            | Command::SetTrackLocked { .. }
            | Command::InsertItem { .. }
            | Command::MoveItem { .. }
            | Command::MoveItems { .. }
            | Command::SplitAt { .. }
            | Command::TrimStart { .. }
            | Command::TrimEnd { .. }
            | Command::Delete { .. }
            | Command::RippleDelete { .. }
            | Command::SetItemParam { .. }
            | Command::SetItemField { .. }
            | Command::SetSpeed { .. }
            | Command::SetCanvasBackground { .. }
            | Command::SetItemParamMany { .. }
            | Command::SetFade { .. }
            | Command::SetLoudness { .. }
            | Command::ClearLoudness { .. }
            | Command::ApplyLoudness { .. }
            | Command::AddTransition { .. }
            | Command::MoveTransition { .. }
            | Command::SetTransitionDuration { .. }
            | Command::RemoveTransition { .. }
            | Command::AddMask { .. }
            | Command::RemoveMask { .. }
            | Command::UpdateMask { .. }
            | Command::ReorderMasks { .. }
            | Command::AddEffect { .. }
            | Command::RemoveEffect { .. }
            | Command::UpdateEffectParam { .. }
            | Command::ReorderEffects { .. }
            | Command::ToggleEffect { .. }
            | Command::SetAnimatedParam { .. }
            | Command::AddKeyframe { .. }
            | Command::RemoveKeyframe { .. }
            | Command::RetimeKeyframe { .. }
            | Command::SetKeyframeHandle { .. }
            | Command::ScaleKeyframeTime { .. }
            | Command::Retranslate { .. }
            | Command::AddBookmark { .. }
            | Command::MoveBookmark { .. }
            | Command::Batch { .. }
            | Command::Undo { .. }
            | Command::Redo { .. }
            | Command::Inspect { .. }
            | Command::Preview { .. }
            | Command::JobStatus { .. }
            | Command::JobCancel { .. }
            | Command::Catalog { .. }
            | Command::ProviderCatalog { .. }
            | Command::FontsList { .. }
            | Command::FontsLoad { .. } => Ok(()),
            // New commands, effects and source paths have no implicit capability.
            _ => Err(EditError::Permission),
        }
    }
}

struct Pending {
    value: Value,
    expires: Instant,
    reply: oneshot::Sender<Result<Value, EditError>>,
}

/// UI-thread inbox. Polling serializes mutations with ordinary desktop commands.
/// Dropped/cancelled/expired requests are discarded before engine admission.
pub struct Inbox {
    receive: mpsc::Receiver<Pending>,
    runner: JobRunner,
    grants: Grants,
}
impl Inbox {
    fn new(grants: Grants, budget: ByteBudget) -> Result<(Handler, Self), CliError> {
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
            budget,
            Arc::new(SystemClock::default()),
        )?;
        Ok((
            Handler { send, full: false },
            Self {
                receive,
                runner,
                grants,
            },
        ))
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

    fn execute(
        &mut self,
        session: &mut Session,
        value: Value,
        publish: impl FnOnce(&project::Project, &project::Metadata) -> engine::Result<()>,
    ) -> engine::Result<Value> {
        let request = crate::build_control_request(session, value)?;
        let caller = Caller {
            principal: "local-mcp",
            external: true,
            policy: &self.grants,
        };
        let response = if request.command == "project.save" {
            session.control_lifecycle_save(request, caller, publish)?
        } else {
            session.control_request(request, caller, &mut self.runner)?
        };
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
    }
}

#[derive(Clone)]
struct Handler {
    send: mpsc::Sender<Pending>,
    full: bool,
}
impl Handler {
    fn tools(&self, start: usize) -> Vec<Tool> {
        let mut envelope = control::contract_schema()
            .get("request")
            .cloned()
            .unwrap_or_else(|| json!({"type":"object"}));
        if let Some(required) = envelope.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|field| {
                field != "api_version" && field != "registry_hash" && field != "session_id"
            });
        }
        let mut tools: Vec<_> = vec![
            tool(
                "command_list",
                "Generated command catalog and current contract",
                json!({"type":"object","properties":{"cursor":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":16}},"additionalProperties":false}),
            ),
            tool(
                "command_run",
                "Run one engine request; mutations require base_rev and idempotency_key",
                envelope.clone(),
            ),
        ].into_iter().skip(start).take(16).collect();
        let remaining = 16 - tools.len();
        for spec in engine::registry()
            .iter()
            .filter(|spec| self.full || CURATED.contains(&spec.id))
            .skip(start.saturating_sub(2))
            .take(remaining)
        {
            let mut schema = envelope.clone();
            let mut params = spec.params_schema.clone();
            if let Some(fields) = params.as_object_mut() {
                fields.remove("$schema");
                if let Some(Value::Object(definitions)) = fields.remove("$defs") {
                    let all = schema.as_object_mut().and_then(|fields| {
                        fields
                            .entry("$defs")
                            .or_insert_with(|| json!({}))
                            .as_object_mut()
                    });
                    if let Some(all) = all {
                        all.extend(definitions);
                    }
                }
            }
            if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
                properties.insert("command".into(), json!({"const":spec.id}));
                properties.insert("params".into(), params);
            }
            if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
                required.retain(|field| field != "command");
                if spec.effect != engine::Effect::Read {
                    required.push(json!("base_rev"));
                    required.push(json!("idempotency_key"));
                }
            }
            tools.push(tool(&spec.id.replace('.', "_"), spec.label, schema));
        }
        tools
    }
}
fn tool(name: &str, description: &str, schema: Value) -> Tool {
    Tool::new(
        name.to_owned(),
        description.to_owned(),
        schema.as_object().cloned().unwrap_or_default(),
    )
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::default();
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("vibecut", env!("CARGO_PKG_VERSION"));
        info.instructions = Some("Use inspect to obtain revision and session identity. Mutation retries must keep the original session_id, base_rev and idempotency_key; never retry with a fresh key after unknown_outcome. Jobs return immediately; poll job_status or call job_cancel.".into());
        info
    }
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let start = request
            .and_then(|request| request.cursor)
            .map(|cursor| cursor.parse::<usize>())
            .transpose()
            .map_err(|_| ErrorData::invalid_params("invalid tool cursor", None))?
            .unwrap_or(0);
        let count = 2 + engine::registry()
            .iter()
            .filter(|spec| self.full || CURATED.contains(&spec.id))
            .count();
        if start > count {
            return Err(ErrorData::invalid_params("invalid tool cursor", None));
        }
        let mut result = ListToolsResult {
            tools: self.tools(start),
            ..Default::default()
        };
        if start + result.tools.len() < count {
            result.next_cursor = Some((start + result.tools.len()).to_string());
        }
        Ok(result)
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = request.arguments.unwrap_or_default();
        if request.name == "command_list" {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Page {
                cursor: Option<usize>,
                limit: Option<usize>,
            }
            let Ok(page) = serde_json::from_value::<Page>(Value::Object(args)) else {
                return Ok(failure("invalid_params").into());
            };
            let start = page.cursor.unwrap_or(0);
            let limit = page.limit.unwrap_or(16);
            let entries = engine::registry();
            if start > entries.len() || limit == 0 || limit > 16 {
                return Ok(failure("invalid_params").into());
            }
            let end = start.saturating_add(limit).min(entries.len());
            let Some(entries) = entries.get(start..end) else {
                return Ok(failure("invalid_params").into());
            };
            let next = if end < engine::registry().len() {
                Some(end)
            } else {
                None
            };
            return Ok(bounded_result(CallToolResult::structured(json!({"api_version":control::API_VERSION,"registry_hash":control::registry_hash(),"limits":control::LIMITS,"entries":entries,"next_cursor":next}))).into());
        }
        let mut value = Value::Object(args);
        if request.name != "command_run" {
            let spec = engine::registry().iter().find(|spec| {
                (self.full || CURATED.contains(&spec.id))
                    && spec.id.replace('.', "_") == request.name
            });
            let Some(spec) = spec else {
                return Err(ErrorData::invalid_params("unknown tool", None));
            };
            if value
                .get("command")
                .is_some_and(|command| command != spec.id)
            {
                return Ok(failure("invalid_params").into());
            }
            value
                .as_object_mut()
                .ok_or_else(|| ErrorData::invalid_params("invalid arguments", None))?
                .insert("command".into(), json!(spec.id));
        }
        if control::bounded_json(&value, control::MAX_FRAME_BYTES).is_err() {
            return Ok(failure("limit").into());
        }
        let (reply, receive) = oneshot::channel();
        if self
            .send
            .try_send(Pending {
                value,
                expires: Instant::now() + DEADLINE,
                reply,
            })
            .is_err()
        {
            return Ok(failure("queue_full").into());
        }
        let result = tokio::select! {
            biased;
            _ = context.ct.cancelled() => failure("cancelled_unknown_outcome"),
            _ = tokio::time::sleep(DEADLINE) => failure("timeout_unknown_outcome"),
            result = receive => match result {
                Ok(Ok(mut value)) => {
                    let images = value.as_object_mut().and_then(|fields| fields.remove("images"));
                    let failed = value.pointer("/value/status").is_some_and(|status| status == "error");
                    let mut result = if failed { CallToolResult::structured_error(value) } else { CallToolResult::structured(value) };
                    if let Some(Value::Array(images)) = images { for image in images {
                        if let Some(data) = image.get("data").and_then(Value::as_str) { result.content.push(ContentBlock::image(data, "image/jpeg")); }
                    } }
                    result
                },
                Ok(Err(error)) => CallToolResult::structured_error(json!({"error":{"code":control::ErrorCode::from_error(&error)}})),
                Err(_) => failure("session_gone_unknown_outcome"),
            }
        };
        Ok(bounded_result(result).into())
    }
}
fn failure(code: &str) -> CallToolResult {
    CallToolResult::structured_error(json!({"error":{"code":code}}))
}

fn bounded_result(result: CallToolResult) -> CallToolResult {
    if control::bounded_json(&result, control::MAX_RESPONSE_BYTES - 4096).is_err() {
        failure("limit")
    } else {
        result
    }
}

/// A listener in the running desktop process. Dropping it closes transports and
/// joins its runtime; queued requests never outlive the UI-owned inbox.
pub struct Server {
    pub inbox: Inbox,
    runtime: Option<tokio::runtime::Runtime>,
}
impl Server {
    pub fn bind(endpoint: &Path, grants: Grants, budget: ByteBudget) -> Result<Self, CliError> {
        let (handler, inbox) = Inbox::new(grants, budget)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let listener = {
            let _enter = runtime.enter();
            socket::listen(endpoint)?
        };
        runtime.spawn(socket::serve(listener, handler));
        Ok(Self {
            inbox,
            runtime: Some(runtime),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

pub(super) fn run(args: Vec<OsString>) -> Result<Output, CliError> {
    let mut project = None;
    let mut attach = false;
    let mut socket = None;
    let mut full = false;
    let mut grants = Grants::default();
    let mut save = true;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--attach") => attach = true,
            Some("--socket") => {
                socket = Some(PathBuf::from(
                    args.next().ok_or(CliError::Rejected("socket required"))?,
                ))
            }
            Some("--output-dir") => {
                grants = Grants::output(Path::new(
                    &args
                        .next()
                        .ok_or(CliError::Rejected("output directory required"))?,
                ))?
            }
            Some("--all-tools") => full = true,
            Some("--no-save") => save = false,
            Some(flag) if flag.starts_with('-') => {
                return Err(CliError::Rejected("unknown MCP flag"));
            }
            _ if project.is_none() => project = Some(PathBuf::from(arg)),
            _ => return Err(CliError::Rejected("one MCP project expected")),
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    if attach {
        if project.is_some() || full || grants.output.is_some() || !save {
            return Err(CliError::Rejected(
                "attach grants belong to the running app",
            ));
        }
        let socket = socket.ok_or(CliError::Rejected("attach requires --socket"))?;
        runtime.block_on(socket::proxy(&socket))?;
    } else {
        if socket.is_some() {
            return Err(CliError::Rejected("headless MCP does not bind a socket"));
        }
        let path = project.ok_or(CliError::Rejected(
            "MCP requires project or --attach --socket",
        ))?;
        let mut lease = project::WriterLease::acquire(&path)?;
        let loaded = lease.load()?;
        let mut session = Session::new(loaded.project, 100)?;
        session.restore_metadata(lease.metadata())?;
        let mut saved_project = session.project().as_ref().clone();
        let mut saved_metadata = session.metadata().clone();
        let (mut handler, mut inbox) =
            Inbox::new(grants, ByteBudget::new(crate::CLI_MEMORY_LIMIT)?)?;
        handler.full = full;
        let worker = std::thread::spawn(move || -> Result<(), CliError> {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<(), CliError> {
                    while !inbox.receive.is_closed() || !inbox.receive.is_empty() {
                        inbox.poll(&mut session, |project, metadata| {
                            let _ = lease.save(project)?;
                            lease.save_metadata(metadata)?;
                            saved_project = project.clone();
                            saved_metadata = metadata.clone();
                            Ok(())
                        })?;
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    if save {
                        let project_dirty = &saved_project != session.project().as_ref();
                        let metadata_dirty = &saved_metadata != session.metadata() || project_dirty;
                        crate::save_control_state(
                            &mut session,
                            &mut lease,
                            project_dirty,
                            metadata_dirty,
                        )?;
                    }
                    Ok(())
                },
            ));
            result.unwrap_or(Err(CliError::Rejected("MCP worker failed")))
        });
        let result = runtime.block_on(async {
            let service = tokio::time::timeout(
                Duration::from_secs(5),
                handler.serve(transport::Framed::new(
                    tokio::io::stdin(),
                    tokio::io::stdout(),
                )),
            )
            .await
            .map_err(|_| CliError::Rejected("MCP initialization timed out"))?
            .map_err(|_| CliError::Rejected("MCP initialization failed"))?;
            service
                .waiting()
                .await
                .map_err(|_| CliError::Rejected("MCP service failed"))?;
            Ok::<_, CliError>(())
        });
        // Shut down SDK handler tasks before waiting for the last sender to drop.
        runtime.shutdown_timeout(Duration::from_secs(1));
        worker
            .join()
            .map_err(|_| CliError::Rejected("MCP worker failed"))??;
        result?;
    }
    Ok(Output {
        value: Value::Null,
        human: String::new(),
        silent: true,
        exit: 0,
    })
}
