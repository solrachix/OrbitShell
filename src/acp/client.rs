use crate::acp::manager::AgentSpec;
use crate::acp::transport::AcpTransport;
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
                .request("initialize", params, DEFAULT_TIMEOUT, Some(&mut noop))?;
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
    ) -> Result<String> {
        if let Some(session_id) = self.session_id.clone() {
            return Ok(session_id);
        }
        let mut noop = |_method: &str, _params: &Value| {};
        let result = self.transport.request(
            "session/new",
            session_new_params(cwd, mcp_servers),
            DEFAULT_TIMEOUT,
            Some(&mut noop),
        )?;
        let session_id = result
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("session/new did not return sessionId"))?
            .to_string();
        self.session_id = Some(session_id.clone());
        Ok(session_id)
    }

    pub fn prompt(
        &self,
        session_id: &str,
        prompt: &str,
        on_update: &mut dyn FnMut(String, bool),
    ) -> Result<Option<String>> {
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

        let result = self.transport.request(
            "session/prompt",
            params,
            Duration::from_secs(120),
            Some(&mut handler),
        )?;

        Ok(extract_result_text(&result))
    }

    pub fn cancel(&self, session_id: &str) -> Result<()> {
        self.transport
            .notify("session/cancel", json!({ "sessionId": session_id }))
    }
}

pub fn session_new_params(cwd: &str, mcp_servers: &[RuntimeMcpServer]) -> Value {
    json!({
        "cwd": cwd,
        "mcpServers": runtime_mcp_servers_value(mcp_servers),
    })
}

fn extract_update_text(params: &Value) -> Option<(String, bool)> {
    if let Some(update) = params.get("update") {
        if let Some(update_kind) = update.get("sessionUpdate").and_then(Value::as_str) {
            if update_kind == "agent_message_chunk" {
                if let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        return Some((text.to_string(), true));
                    }
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
        params.pointer("/update/content/text"),
        params.pointer("/update/message"),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Some(text) = candidate.as_str() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some((trimmed.to_string(), false));
            }
        }
    }
    None
}

fn extract_result_text(result: &Value) -> Option<String> {
    let candidates = [
        result.get("text"),
        result.get("message"),
        result.pointer("/response/text"),
        result.pointer("/content/text"),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Some(text) = candidate.as_str() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}
