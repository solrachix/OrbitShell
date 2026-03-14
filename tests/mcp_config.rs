#[test]
fn mcp_server_round_trips_with_required_fields() {
    let server = orbitshell::mcp::config::McpServerConfig {
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
    };
    let json = serde_json::to_string(&server).unwrap();
    let decoded: orbitshell::mcp::config::McpServerConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.id, "fs");
}

fn stdio_server(id: &str, name: &str, command: &str) -> orbitshell::mcp::config::McpServerConfig {
    orbitshell::mcp::config::McpServerConfig {
        id: id.into(),
        name: name.into(),
        transport: "stdio".into(),
        command: Some(command.into()),
        url: None,
        args: vec![".".into()],
        env: Default::default(),
        enabled: true,
        last_tested_at: None,
        last_status: None,
        last_error: None,
    }
}

#[test]
fn mcp_config_can_add_edit_and_delete_servers() {
    let mut config = orbitshell::mcp::config::GlobalMcpConfig::default();
    config.upsert_server(stdio_server("fs", "Filesystem", "mcp-server-fs"));
    config.upsert_server(stdio_server("git", "Git", "mcp-server-git"));

    assert_eq!(config.servers.len(), 2);

    let mut edited = stdio_server("fs", "Filesystem Local", "mcp-server-fs");
    edited.enabled = false;
    config.upsert_server(edited);

    assert_eq!(config.servers.len(), 2);
    assert_eq!(config.servers[0].name, "Filesystem Local");
    assert!(!config.servers[0].enabled);

    assert!(config.remove_server("git"));
    assert_eq!(config.servers.len(), 1);
    assert_eq!(config.servers[0].id, "fs");
}

#[test]
fn mcp_config_applies_probe_status_updates() {
    let mut config = orbitshell::mcp::config::GlobalMcpConfig {
        servers: vec![stdio_server("fs", "Filesystem", "mcp-server-fs")],
    };

    let updated = config.apply_probe_result(
        "fs",
        &orbitshell::mcp::probe::McpProbeResult {
            tested_at: 1234,
            status: "online".into(),
            error: None,
        },
    );

    assert!(updated);
    let server = &config.servers[0];
    assert_eq!(server.last_tested_at, Some(1234));
    assert_eq!(server.last_status.as_deref(), Some("online"));
    assert_eq!(server.last_error, None);
}

#[test]
fn url_backed_mcp_server_round_trips() {
    let server = orbitshell::mcp::config::McpServerConfig {
        id: "remote".into(),
        name: "Remote MCP".into(),
        transport: "http".into(),
        command: None,
        url: Some("http://127.0.0.1:8123/mcp".into()),
        args: Vec::new(),
        env: Default::default(),
        enabled: true,
        last_tested_at: Some(99),
        last_status: Some("offline".into()),
        last_error: Some("connection refused".into()),
    };

    let json = serde_json::to_string(&server).unwrap();
    let decoded: orbitshell::mcp::config::McpServerConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.url.as_deref(), Some("http://127.0.0.1:8123/mcp"));
    assert_eq!(decoded.last_status.as_deref(), Some("offline"));
}
