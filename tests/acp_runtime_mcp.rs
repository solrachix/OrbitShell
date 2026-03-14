use orbitshell::acp::client::session_new_params;
use orbitshell::acp::manager::AgentSpec;
use orbitshell::acp::resolve::{
    AgentCandidate, AgentKey, AgentSourceKind, ConflictPolicy, resolve_agent,
    resolve_effective_agents,
};
use orbitshell::mcp::config::{GlobalMcpConfig, McpServerConfig};
use orbitshell::mcp::probe::resolve_runtime_mcp_servers;

fn agent_spec(id: &str, name: &str, command: &str) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        name: name.into(),
        command: command.into(),
        args: Vec::new(),
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
    let params = session_new_params("C:/repo", &runtime);
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
