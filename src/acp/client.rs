use crate::acp::manager::AgentSpec;
use crate::acp::model_discovery::AcpModelOption;
use crate::acp::transport::{AcpTransport, IncomingRequest};
use crate::mcp::probe::{RuntimeMcpServer, runtime_mcp_servers_value};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::time::Duration;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(45);

pub struct AcpClient {
    transport: AcpTransport,
    pub protocol_version: Option<String>,
    pub agent_info: Option<Value>,
    pub agent_capabilities: Option<Value>,
    pub session_id: Option<String>,
    pub session_model_options: Vec<AcpModelOption>,
}

pub struct SessionBootstrap {
    pub session_id: String,
    pub model_options: Vec<AcpModelOption>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpResponseText {
    Plain(String),
    Markdown(String),
}

impl AcpResponseText {
    pub fn text(&self) -> &str {
        match self {
            AcpResponseText::Plain(text) | AcpResponseText::Markdown(text) => text,
        }
    }

    pub fn is_markdown(&self) -> bool {
        matches!(self, AcpResponseText::Markdown(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequest {
    pub request_id: u64,
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub tool_name: Option<String>,
    pub options: Vec<PermissionOption>,
    pub raw_params: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Selected { option_id: String },
    Cancelled,
}

impl AcpClient {
    pub fn connect(spec: &AgentSpec) -> Result<Self> {
        let transport = AcpTransport::spawn(spec)?;
        Ok(Self {
            transport,
            protocol_version: None,
            agent_info: None,
            agent_capabilities: None,
            session_id: None,
            session_model_options: Vec::new(),
        })
    }

    pub fn initialize(&mut self) -> Result<()> {
        let params = json!({
            "protocolVersion": "0.2",
            "clientCapabilities": {
                "terminal": true,
                "fs": {
                    "readTextFile": true,
                    "writeTextFile": true
                }
            },
            "clientInfo": {
                "name": "orbitshell",
                "title": "OrbitShell",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        let mut noop = |_method: &str, _params: &Value| {};
        let result =
            self.transport
                .request("initialize", params, DEFAULT_TIMEOUT, Some(&mut noop), None)?;
        self.protocol_version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        self.agent_info = result.get("agentInfo").cloned();
        self.agent_capabilities = result.get("agentCapabilities").cloned();
        Ok(())
    }

    pub fn ensure_session(
        &mut self,
        cwd: &str,
        mcp_servers: &[RuntimeMcpServer],
        selected_model: Option<&str>,
    ) -> Result<SessionBootstrap> {
        if let Some(session_id) = self.session_id.clone() {
            return Ok(SessionBootstrap {
                session_id,
                model_options: self.session_model_options.clone(),
            });
        }
        let mut noop = |_method: &str, _params: &Value| {};
        let result = self.transport.request(
            "session/new",
            session_new_params(cwd, mcp_servers, selected_model),
            DEFAULT_TIMEOUT,
            Some(&mut noop),
            None,
        )?;
        let session_id = result
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("session/new did not return sessionId"))?
            .to_string();
        let model_options = extract_model_options_from_session_new(&result);
        self.session_id = Some(session_id.clone());
        self.session_model_options = model_options.clone();
        Ok(SessionBootstrap {
            session_id,
            model_options,
        })
    }

    pub fn set_session_config_option(
        &self,
        session_id: &str,
        option_id: &str,
        value: &str,
    ) -> Result<()> {
        let mut noop = |_method: &str, _params: &Value| {};
        self.transport.request(
            "session/set_config_option",
            json!({
                "sessionId": session_id,
                "optionId": option_id,
                "type": "select",
                "value": value
            }),
            DEFAULT_TIMEOUT,
            Some(&mut noop),
            None,
        )?;
        Ok(())
    }

    pub fn prompt(
        &self,
        session_id: &str,
        prompt: &str,
        on_update: &mut dyn FnMut(String, bool),
        on_permission_request: &mut dyn FnMut(PermissionRequest) -> Result<PermissionDecision>,
    ) -> Result<Option<AcpResponseText>> {
        let params = json!({
            "sessionId": session_id,
            "prompt": [
                {
                    "type": "text",
                    "text": prompt
                }
            ]
        });

        let mut handler = |method: &str, params: &Value| {
            if method == "stderr" {
                if let Some(line) = params.as_str() {
                    on_update(format!("[agent stderr] {line}"), false);
                }
                return;
            }

            if method != "session/update" {
                return;
            }

            if let Some((text, append_to_last)) = extract_update_text(params) {
                on_update(text, append_to_last);
            }
        };

        let transport = &self.transport;
        let mut request_handler = move |request: &IncomingRequest| -> Result<()> {
            let Some(permission_request) =
                extract_permission_request(&request.method, request.id, &request.params)
            else {
                return Err(anyhow!(
                    "unsupported inbound ACP request '{}'",
                    request.method
                ));
            };

            let decision = on_permission_request(permission_request)?;
            let payload = permission_response_payload(&decision);
            transport.respond_result(request.id, payload)
        };

        let result = self.transport.request(
            "session/prompt",
            params,
            Duration::from_secs(120),
            Some(&mut handler),
            Some(&mut request_handler),
        )?;

        Ok(extract_result_text(&result))
    }

    pub fn cancel(&self, session_id: &str) -> Result<()> {
        self.transport
            .notify("session/cancel", json!({ "sessionId": session_id }))
    }
}

pub fn session_new_params(
    cwd: &str,
    mcp_servers: &[RuntimeMcpServer],
    selected_model: Option<&str>,
) -> Value {
    let mut params = json!({
        "cwd": cwd,
        "mcpServers": runtime_mcp_servers_value(mcp_servers),
    });
    if let Some(model_id) = selected_model {
        if let Some(object) = params.as_object_mut() {
            object.insert("model".into(), Value::String(model_id.into()));
        }
    }
    params
}

pub fn extract_model_options_from_session_new(result: &Value) -> Vec<AcpModelOption> {
    let Some(options) = result.get("configOptions").and_then(Value::as_array) else {
        return Vec::new();
    };

    options
        .iter()
        .find(|option| {
            option.get("category").and_then(Value::as_str) == Some("model")
                && option.get("type").and_then(Value::as_str) == Some("select")
        })
        .and_then(|option| {
            let current_value = option.get("currentValue").and_then(Value::as_str);
            option
                .get("options")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|entry| {
                            let id = entry.get("value").and_then(Value::as_str)?.to_string();
                            let label = entry
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or(&id)
                                .to_string();
                            let description = entry
                                .get("description")
                                .and_then(Value::as_str)
                                .map(ToString::to_string);

                            Some(AcpModelOption {
                                is_default: current_value == Some(id.as_str()),
                                id,
                                label,
                                description,
                            })
                        })
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_default()
}

fn extract_update_text(params: &Value) -> Option<(String, bool)> {
    if let Some(update) = params.get("update") {
        if let Some(update_kind) = update.get("sessionUpdate").and_then(Value::as_str) {
            let append_to_last = update_kind.contains("chunk") || update_kind.contains("delta");
            let chunk_candidates = [
                update.get("delta"),
                update.get("message"),
                update.get("content"),
                update.pointer("/content/text"),
                update.pointer("/message/content"),
                update.pointer("/message/text"),
            ];
            for candidate in chunk_candidates.into_iter().flatten() {
                if let Some(text) = extract_text_content(candidate) {
                    return Some((text, append_to_last));
                }
            }
        }
    }

    let candidates = [
        params.get("delta"),
        params.get("text"),
        params.get("message"),
        params.pointer("/output/text"),
        params.pointer("/content/text"),
        params.get("content"),
        params.pointer("/update/content/text"),
        params.pointer("/update/content"),
        params.pointer("/update/message"),
        params.pointer("/message/content"),
        params.pointer("/message/text"),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Some(text) = extract_text_content(candidate) {
            return Some((text, false));
        }
    }
    None
}

fn extract_result_text(result: &Value) -> Option<AcpResponseText> {
    let candidates = [
        result.get("text"),
        result.get("message"),
        result.pointer("/response/text"),
        result.pointer("/content/text"),
        result.get("content"),
        result.pointer("/message/content"),
    ];
    for candidate in candidates.into_iter().flatten() {
        // For arrays (content: [...]), always join items with \n to preserve
        // structure like fences, lists and per-file entries from the agent.
        if let Value::Array(items) = candidate {
            if let Some(text) = extract_array_text_content(items, "\n") {
                return Some(classify_response_text(text));
            }
            continue;
        }
        if let Some(text) = extract_text_content(candidate) {
            return Some(classify_response_text(text));
        }
    }
    None
}

fn extract_text_content(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => (!text.trim().is_empty()).then(|| text.to_string()),
        Value::Array(items) => extract_array_text_content(items, ""),
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(extract_text_content) {
                return Some(text);
            }
            if let Some(content) = map.get("content").and_then(extract_text_content) {
                return Some(content);
            }
            if let Some(message) = map.get("message").and_then(extract_text_content) {
                return Some(message);
            }
            if let Some(delta) = map.get("delta").and_then(extract_text_content) {
                return Some(delta);
            }
            None
        }
        _ => None,
    }
}

fn extract_array_text_content(items: &[Value], separator: &str) -> Option<String> {
    let mut combined = String::new();
    let mut first = true;
    for item in items {
        if let Some(text) = extract_text_content(item) {
            if !first && !separator.is_empty() {
                // Only insert separator if previous content doesn't already
                // end with it (avoids double-newlines for items like "ls\n").
                if !(separator == "\n" && combined.ends_with('\n')) {
                    combined.push_str(separator);
                }
            }
            combined.push_str(&text);
            first = false;
        }
    }
    normalize_extracted_text(&combined)
}

fn normalize_extracted_text(text: &str) -> Option<String> {
    (!text.trim().is_empty()).then(|| text.to_string())
}

fn classify_response_text(text: String) -> AcpResponseText {
    if looks_like_markdown(&text) {
        AcpResponseText::Markdown(text)
    } else {
        AcpResponseText::Plain(text)
    }
}

fn looks_like_markdown(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.starts_with("```") {
        return true;
    }

    // Line-level markdown constructs (strongest signal).
    let has_block_construct = trimmed.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with('#')
            || line.starts_with("- ")
            || line.starts_with("* ")
            || line.starts_with("+ ")
            || is_ordered_list_line(line)
            || line.starts_with('>')
    });
    if has_block_construct {
        return true;
    }

    // Paired inline markers (require opening AND closing).
    if trimmed.contains("**") {
        return true;
    }
    // Inline code: require at least one complete `...` pair.
    if let Some(first) = trimmed.find('`') {
        if trimmed[first + 1..].contains('`') {
            return true;
        }
    }
    // Markdown links: [label](url)
    if trimmed.contains('[') && trimmed.contains("](") {
        return true;
    }

    false
}

fn is_ordered_list_line(line: &str) -> bool {
    let mut chars = line.chars().peekable();
    let mut saw_digit = false;
    while let Some(ch) = chars.peek().copied() {
        if ch.is_ascii_digit() {
            saw_digit = true;
            chars.next();
            continue;
        }
        break;
    }
    saw_digit && matches!(chars.next(), Some('.')) && matches!(chars.next(), Some(' '))
}

pub fn extract_permission_request(
    method: &str,
    request_id: u64,
    params: &Value,
) -> Option<PermissionRequest> {
    if method != "session/request_permission" {
        return None;
    }

    let options = params
        .get("options")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = item.get("id").and_then(Value::as_str)?.to_string();
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    let description = item
                        .get("description")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    Some(PermissionOption {
                        id,
                        name,
                        description,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(PermissionRequest {
        request_id,
        session_id: params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        title: pick_string_field(params, &["title", "name"]),
        description: pick_string_field(params, &["description", "detail", "details"]),
        tool_name: pick_nested_string_field(params, &["tool", "action"], &["name", "id"]),
        options,
        raw_params: params.clone(),
    })
}

pub fn permission_response_payload(decision: &PermissionDecision) -> Value {
    match decision {
        PermissionDecision::Selected { option_id } => json!({
            "outcome": "selected",
            "optionId": option_id
        }),
        PermissionDecision::Cancelled => json!({
            "outcome": "cancelled"
        }),
    }
}

fn pick_string_field(params: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        params
            .get(key)
            .and_then(Value::as_str)
            .map(ToString::to_string)
    })
}

fn pick_nested_string_field(params: &Value, parents: &[&str], keys: &[&str]) -> Option<String> {
    parents.iter().find_map(|parent| {
        let object = params.get(parent)?.as_object()?;
        keys.iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str))
            .map(ToString::to_string)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AcpResponseText, PermissionDecision, extract_permission_request, extract_result_text,
        extract_text_content, extract_update_text, permission_response_payload,
    };
    use serde_json::json;

    #[test]
    fn extract_update_text_reads_chunked_content_arrays() {
        let params = json!({
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": [
                    { "type": "text", "text": "Tudo " },
                    { "type": "text", "text": "certo" }
                ]
            }
        });

        assert_eq!(
            extract_update_text(&params),
            Some(("Tudo certo".to_string(), true))
        );
    }

    #[test]
    fn extract_update_text_reads_nested_message_content() {
        let params = json!({
            "message": {
                "content": [
                    { "type": "text", "text": "Resposta" },
                    { "type": "text", "text": " parcial" }
                ]
            }
        });

        assert_eq!(
            extract_update_text(&params),
            Some(("Resposta parcial".to_string(), false))
        );
    }

    #[test]
    fn extract_result_text_reads_content_arrays() {
        let result = json!({
            "content": [
                { "type": "text", "text": "Final" },
                { "type": "text", "text": "ok" }
            ]
        });

        assert_eq!(
            extract_result_text(&result),
            Some(AcpResponseText::Plain("Final\nok".to_string()))
        );
    }

    #[test]
    fn extract_text_content_ignores_empty_strings() {
        assert_eq!(extract_text_content(&json!("   ")), None);
    }

    #[test]
    fn extract_update_text_preserves_streaming_whitespace_for_prefix_matching() {
        let params = json!({
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": [
                    { "type": "text", "text": "```sh\n" },
                    { "type": "text", "text": "    Directory: C:\\repo\n" },
                    { "type": "text", "text": "    assets\n" }
                ]
            }
        });

        assert_eq!(
            extract_update_text(&params),
            Some((
                "```sh\n    Directory: C:\\repo\n    assets\n".to_string(),
                true
            ))
        );
    }

    #[test]
    fn extract_result_text_preserves_markdown_fence_whitespace() {
        let result = json!({
            "content": [
                { "type": "text", "text": "```sh\n" },
                { "type": "text", "text": "ls\n" },
                { "type": "text", "text": "```" }
            ]
        });

        assert_eq!(
            extract_result_text(&result),
            Some(AcpResponseText::Markdown("```sh\nls\n```".to_string()))
        );
    }

    #[test]
    fn extract_result_text_reconstructs_markdown_block_boundaries_from_content_arrays() {
        let result = json!({
            "content": [
                { "type": "text", "text": "```sh" },
                { "type": "text", "text": "ls" },
                { "type": "text", "text": "```" }
            ]
        });

        assert_eq!(
            extract_result_text(&result),
            Some(AcpResponseText::Markdown("```sh\nls\n```".to_string()))
        );
    }

    #[test]
    fn extract_result_text_detects_markdown_heading_and_list_content() {
        let result = json!({
            "content": [
                { "type": "text", "text": "# Heading\n" },
                { "type": "text", "text": "- item\n" }
            ]
        });

        assert_eq!(
            extract_result_text(&result),
            Some(AcpResponseText::Markdown("# Heading\n- item\n".to_string()))
        );
    }

    #[test]
    fn extract_permission_request_reads_core_metadata_and_options() {
        let params = json!({
            "sessionId": "sess_123",
            "title": "Approve tool call",
            "description": "The agent wants to edit a file.",
            "tool": {
                "name": "writeTextFile"
            },
            "options": [
                {
                    "id": "allow_once",
                    "name": "Permitir uma vez",
                    "description": "Allow this action once"
                },
                {
                    "id": "allow_always",
                    "name": "Sempre permitir",
                    "description": "Always allow this action"
                },
                {
                    "id": "reject",
                    "name": "Negar",
                    "description": "Reject this action"
                }
            ]
        });

        let request = extract_permission_request("session/request_permission", 42, &params)
            .expect("permission request should parse");

        assert_eq!(request.request_id, 42);
        assert_eq!(request.session_id.as_deref(), Some("sess_123"));
        assert_eq!(request.title.as_deref(), Some("Approve tool call"));
        assert_eq!(
            request.description.as_deref(),
            Some("The agent wants to edit a file.")
        );
        assert_eq!(request.options.len(), 3);
        assert_eq!(request.options[0].id, "allow_once");
        assert_eq!(request.options[1].name, "Sempre permitir");
    }

    #[test]
    fn permission_response_payload_maps_selected_and_cancelled() {
        let selected = permission_response_payload(&PermissionDecision::Selected {
            option_id: "allow_once".into(),
        });
        assert_eq!(selected["outcome"], "selected");
        assert_eq!(selected["optionId"], "allow_once");

        let cancelled = permission_response_payload(&PermissionDecision::Cancelled);
        assert_eq!(cancelled["outcome"], "cancelled");
        assert!(cancelled.get("optionId").is_none());
    }
}
