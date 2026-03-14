use crate::mcp::config::{GlobalMcpConfig, McpServerConfig};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeMcpServer {
    pub id: String,
    pub transport: String,
    pub command: Option<String>,
    pub url: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

pub fn resolve_runtime_mcp_servers(config: &GlobalMcpConfig) -> Vec<RuntimeMcpServer> {
    config
        .servers
        .iter()
        .filter(|server| server.enabled)
        .filter_map(RuntimeMcpServer::from_config)
        .collect()
}

pub fn load_enabled_runtime_mcp_servers() -> Vec<RuntimeMcpServer> {
    GlobalMcpConfig::load_default()
        .map(|config| resolve_runtime_mcp_servers(&config))
        .unwrap_or_default()
}

pub fn runtime_mcp_servers_value(servers: &[RuntimeMcpServer]) -> Value {
    Value::Array(servers.iter().map(runtime_mcp_server_value).collect())
}

impl RuntimeMcpServer {
    fn from_config(config: &McpServerConfig) -> Option<Self> {
        let has_stdio_command =
            matches!(config.transport.as_str(), "stdio") && config.command.is_some();
        let has_http_url =
            matches!(config.transport.as_str(), "http" | "sse") && config.url.is_some();
        if !(has_stdio_command || has_http_url) {
            return None;
        }

        Some(Self {
            id: config.id.clone(),
            transport: config.transport.clone(),
            command: config.command.clone(),
            url: config.url.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
        })
    }
}

fn runtime_mcp_server_value(server: &RuntimeMcpServer) -> Value {
    json!({
        "id": server.id,
        "transport": server.transport,
        "command": server.command,
        "url": server.url,
        "args": server.args,
        "env": server.env,
    })
}
