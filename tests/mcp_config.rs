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
