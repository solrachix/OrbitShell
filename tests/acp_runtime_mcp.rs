use std::collections::BTreeMap;

use orbitshell::acp::client::{extract_model_options_from_session_new, session_new_params};
use orbitshell::acp::manager::AgentSpec;
use orbitshell::acp::resolve::{
    AgentCandidate, AgentKey, AgentSourceKind, ConflictPolicy, resolve_agent,
    resolve_effective_agents,
};
use orbitshell::mcp::config::{GlobalMcpConfig, McpServerConfig};
use orbitshell::mcp::probe::resolve_runtime_mcp_servers;
use serde_json::json;

fn agent_spec(id: &str, name: &str, command: &str) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        name: name.into(),
        command: command.into(),
        args: Vec::new(),
        fixed_env: BTreeMap::new(),
        env_keys: Vec::new(),
        install: None,
        auth: None,
    }
}

#[test]
fn session_new_uses_enabled_global_mcp_servers() {
    let config = GlobalMcpConfig {
        servers: vec![
            McpServerConfig {
                id: "fs".into(),
                name: "Filesystem".into(),
                transport: "stdio".into(),
                command: Some("mcp-server-fs".into()),
                url: None,
                args: vec![".".into()],
                env: Default::default(),
                enabled: true,
                last_tested_at: None,
                last_status: None,
                last_error: None,
            },
            McpServerConfig {
                id: "disabled".into(),
                name: "Disabled".into(),
                transport: "http".into(),
                command: None,
                url: Some("http://localhost:3000".into()),
                args: Vec::new(),
                env: Default::default(),
                enabled: false,
                last_tested_at: None,
                last_status: None,
                last_error: None,
            },
        ],
    };

    let runtime = resolve_runtime_mcp_servers(&config);
    let params = session_new_params("C:/repo", &runtime, None);
    let servers = params
        .get("mcpServers")
        .and_then(|value| value.as_array())
        .expect("mcpServers array");

    assert_eq!(servers.len(), 1);
    assert_eq!(
        servers[0].get("id").and_then(|value| value.as_str()),
        Some("fs")
    );
}

#[test]
fn session_new_includes_selected_model_override() {
    let runtime = resolve_runtime_mcp_servers(&GlobalMcpConfig { servers: vec![] });
    let params = session_new_params("C:/repo", &runtime, Some("gpt-5.4"));
    assert_eq!(
        params.get("model").and_then(|value| value.as_str()),
        Some("gpt-5.4")
    );
}

#[test]
fn session_new_omits_model_when_not_selected() {
    let runtime = resolve_runtime_mcp_servers(&GlobalMcpConfig { servers: vec![] });
    let params = session_new_params("C:/repo", &runtime, None);
    assert_eq!(params.get("model"), None);
}

#[test]
fn session_new_config_options_extract_model_selector() {
    let result = json!({
        "sessionId": "sess_123",
        "configOptions": [
            {
                "id": "mode",
                "category": "mode",
                "type": "select",
                "currentValue": "ask",
                "options": [
                    { "value": "ask", "name": "Ask" }
                ]
            },
            {
                "id": "model",
                "name": "Model",
                "category": "model",
                "type": "select",
                "currentValue": "gpt-5.4",
                "options": [
                    { "value": "gpt-5.3", "name": "GPT-5.3", "description": "Stable" },
                    { "value": "gpt-5.4", "name": "GPT-5.4", "description": "Best" }
                ]
            }
        ]
    });

    let models = extract_model_options_from_session_new(&result);

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "gpt-5.3");
    assert_eq!(models[0].label, "GPT-5.3");
    assert!(!models[0].is_default);
    assert_eq!(models[1].id, "gpt-5.4");
    assert!(models[1].is_default);
}

#[test]
fn session_new_config_options_ignore_non_model_selectors() {
    let result = json!({
        "sessionId": "sess_123",
        "configOptions": [
            {
                "id": "mode",
                "category": "mode",
                "type": "select",
                "currentValue": "ask",
                "options": [
                    { "value": "ask", "name": "Ask" }
                ]
            }
        ]
    });

    let models = extract_model_options_from_session_new(&result);

    assert!(models.is_empty());
}

#[test]
fn resolve_agent_returns_source_selected_spec_for_runtime() {
    let rows = resolve_effective_agents(
        vec![
            AgentCandidate {
                agent_key: AgentKey {
                    source_type: AgentSourceKind::Registry,
                    source_id: "official".into(),
                    acp_id: "codex".into(),
                },
                spec: agent_spec("codex", "Codex Registry", "codex-acp"),
                managed_state: None,
            },
            AgentCandidate {
                agent_key: AgentKey {
                    source_type: AgentSourceKind::WorkspaceCustom,
                    source_id: "workspace".into(),
                    acp_id: "codex".into(),
                },
                spec: agent_spec("codex", "Codex Workspace", "codex-local"),
                managed_state: None,
            },
        ],
        ConflictPolicy::LocalWins,
    );

    let selected = rows.iter().find(|row| row.is_selected).unwrap();
    let spec = resolve_agent(&rows, &selected.agent_key).expect("resolved agent");

    assert_eq!(spec.command, "codex-local");
}
