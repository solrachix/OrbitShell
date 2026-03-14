use orbitshell::acp::install::state::ManagedAgentState;
use orbitshell::acp::manager::AgentSpec;
use orbitshell::acp::resolve::{
    AgentCandidate, AgentKey, AgentSourceKind, ConflictPolicy, list_alternate_sources,
    resolve_agent, resolve_effective_agents,
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

fn managed_candidate(source_id: &str, spec: AgentSpec, installed_version: &str) -> AgentCandidate {
    AgentCandidate {
        agent_key: AgentKey {
            source_type: AgentSourceKind::Registry,
            source_id: source_id.into(),
            acp_id: spec.id.clone(),
        },
        spec,
        managed_state: Some(ManagedAgentState {
            id: "codex".into(),
            installed_version: Some(installed_version.into()),
            ..Default::default()
        }),
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
    let alternate_spec = resolve_agent(&rows, &alternate.agent_key).unwrap();
    assert_eq!(alternate_spec.command, "codex-global");
}

#[test]
fn local_wins_prefers_global_custom_over_registry_managed() {
    let rows = resolve_effective_agents(
        vec![
            managed_candidate(
                "official",
                agent_spec("codex", "Codex Registry", "codex-acp"),
                "0.9.0",
            ),
            candidate(
                AgentSourceKind::GlobalCustom,
                "global",
                agent_spec("codex", "Codex Global", "codex-global"),
            ),
        ],
        ConflictPolicy::LocalWins,
    );

    let selected = rows.iter().find(|row| row.is_selected).unwrap();
    assert_eq!(selected.source_type, AgentSourceKind::GlobalCustom);

    let alternates = list_alternate_sources(&rows, "codex");
    assert_eq!(alternates.len(), 1);
    assert_eq!(alternates[0].source_type, AgentSourceKind::Registry);
}

#[test]
fn local_wins_prefers_workspace_custom_over_registry_managed() {
    let rows = resolve_effective_agents(
        vec![
            managed_candidate(
                "official",
                agent_spec("codex", "Codex Registry", "codex-acp"),
                "0.9.0",
            ),
            candidate(
                AgentSourceKind::WorkspaceCustom,
                "workspace",
                agent_spec("codex", "Codex Workspace", "codex-local"),
            ),
        ],
        ConflictPolicy::LocalWins,
    );

    let selected = rows.iter().find(|row| row.is_selected).unwrap();
    assert_eq!(selected.source_type, AgentSourceKind::WorkspaceCustom);

    let alternate = &list_alternate_sources(&rows, "codex")[0];
    let alternate_spec = resolve_agent(&rows, &alternate.agent_key).unwrap();
    assert_eq!(alternate_spec.command, "codex-acp");
}
