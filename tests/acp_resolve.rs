use orbitshell::acp::manager::AgentSpec;
use orbitshell::acp::resolve::{
    AgentCandidate, AgentKey, AgentSourceKind, ConflictPolicy, resolve_effective_agents,
};

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

fn candidate(source_type: AgentSourceKind, source_id: &str, spec: AgentSpec) -> AgentCandidate {
    AgentCandidate {
        agent_key: AgentKey {
            source_type,
            source_id: source_id.into(),
            acp_id: spec.id.clone(),
        },
        spec,
        managed_state: None,
    }
}

#[test]
fn show_both_keeps_two_rows_with_distinct_agent_keys() {
    let rows = resolve_effective_agents(
        vec![
            candidate(
                AgentSourceKind::Registry,
                "official",
                agent_spec("codex", "Codex Registry", "codex-acp"),
            ),
            candidate(
                AgentSourceKind::WorkspaceCustom,
                "workspace",
                agent_spec("codex", "Codex Workspace", "codex-local"),
            ),
        ],
        ConflictPolicy::ShowBoth,
    );

    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].agent_key, rows[1].agent_key);
    assert!(rows.iter().all(|row| row.is_selected));
}

#[test]
fn registry_wins_keeps_losing_source_reachable() {
    let rows = resolve_effective_agents(
        vec![
            candidate(
                AgentSourceKind::Registry,
                "official",
                agent_spec("codex", "Codex Registry", "codex-acp"),
            ),
            candidate(
                AgentSourceKind::GlobalCustom,
                "global",
                agent_spec("codex", "Codex Global", "codex-global"),
            ),
        ],
        ConflictPolicy::RegistryWins,
    );

    assert_eq!(rows.len(), 2);
    let selected = rows.iter().find(|row| row.is_selected).unwrap();
    let alternate = rows.iter().find(|row| !row.is_selected).unwrap();

    assert_eq!(selected.source_type, AgentSourceKind::Registry);
    assert!(alternate.is_alternate);
    assert_eq!(
        alternate.agent_key.source_type,
        AgentSourceKind::GlobalCustom
    );
}
