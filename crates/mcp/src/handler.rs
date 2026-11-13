//! Serves the MCP tool surface over the inbox queue.
use super::{DEADLINE, Pending};
use engine::Failure;
use engine::control::{self};
use rmcp::{
    ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
        Implementation, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
        ServerCapabilities, ServerConfig, Tool,
    },
    service::{RequestContext, RoleServer},
};
use serde_json::{Value, json};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub struct Handler {
    pub(super) project_routing: bool,
    pub(super) send: mpsc::Sender<Pending>,
    pub(super) full: bool,
}
impl Handler {
    pub(super) fn new(send: mpsc::Sender<Pending>) -> Self {
        Self {
            send,
            full: false,
            project_routing: false,
        }
    }

    /// Serve every registry command as a tool, not only the curated surface.
    pub fn all_tools(&mut self) {
        self.full = true;
    }

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
        if self.project_routing
            && let Some(properties) = envelope
                .get_mut("properties")
                .and_then(Value::as_object_mut)
        {
            properties.insert("project_id".into(), json!({"type":"string", "pattern":"^[0-9]+$", "description":"Desktop project window; omitted selects the focused editor"}));
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
            .filter(|spec| self.full || spec.curated)
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
            .filter(|spec| self.full || spec.curated)
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
                (self.full || spec.curated) && spec.id.replace('.', "_") == request.name
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
                Ok(Err(error)) => CallToolResult::structured_error(json!({"error":{"code":error.code()}})),
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
