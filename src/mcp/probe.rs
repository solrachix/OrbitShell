use crate::mcp::config::{GlobalMcpConfig, McpServerConfig};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeMcpServer {
    pub id: String,
    pub transport: String,
    pub command: Option<String>,
    pub url: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpProbeResult {
    pub tested_at: i64,
    pub status: String,
    pub error: Option<String>,
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

pub fn probe_server_config(config: &McpServerConfig) -> McpProbeResult {
    let tested_at = current_timestamp();
    let runtime = match RuntimeMcpServer::from_config(config) {
        Some(runtime) => runtime,
        None => {
            return McpProbeResult {
                tested_at,
                status: "misconfigured".into(),
                error: Some("transport requires command or url".into()),
            };
        }
    };

    match runtime.transport.as_str() {
        "stdio" => {
            let Some(command) = runtime.command.as_deref() else {
                return McpProbeResult {
                    tested_at,
                    status: "misconfigured".into(),
                    error: Some("stdio transport requires command".into()),
                };
            };

            if command_exists(command) {
                McpProbeResult {
                    tested_at,
                    status: "online".into(),
                    error: None,
                }
            } else {
                McpProbeResult {
                    tested_at,
                    status: "offline".into(),
                    error: Some(format!("command '{command}' not found")),
                }
            }
        }
        "http" | "sse" => {
            let Some(url) = runtime.url.as_deref() else {
                return McpProbeResult {
                    tested_at,
                    status: "misconfigured".into(),
                    error: Some("http/sse transport requires url".into()),
                };
            };

            match ureq::get(url).call() {
                Ok(_) | Err(ureq::Error::Status(_, _)) => McpProbeResult {
                    tested_at,
                    status: "online".into(),
                    error: None,
                },
                Err(err) => McpProbeResult {
                    tested_at,
                    status: "offline".into(),
                    error: Some(err.to_string()),
                },
            }
        }
        _ => McpProbeResult {
            tested_at,
            status: "misconfigured".into(),
            error: Some(format!("unsupported transport '{}'", runtime.transport)),
        },
    }
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

fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn command_exists(command: &str) -> bool {
    let probe = if cfg!(windows) { "where" } else { "which" };
    for candidate in resolve_command_candidates(command) {
        if Command::new(probe)
            .arg(&candidate)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn resolve_command_candidates(command: &str) -> Vec<String> {
    if !cfg!(windows) {
        return vec![command.to_string()];
    }

    let path = Path::new(command);
    if path.extension().is_some() {
        return vec![command.to_string()];
    }

    vec![
        command.to_string(),
        format!("{command}.cmd"),
        format!("{command}.exe"),
    ]
}
