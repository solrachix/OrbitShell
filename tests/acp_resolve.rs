use orbitshell::acp::install::state::ManagedAgentState;
use orbitshell::acp::manager::AgentSpec;
use orbitshell::acp::registry::fetch::CachedRegistryData;
use orbitshell::acp::registry::model::{
    RegistryCatalogEntry, RegistryDistribution, RegistryManifest, RegistryPackageDistribution,
};
use orbitshell::acp::resolve::{
    AgentCandidate, AgentKey, AgentSourceKind, CatalogFilter, CatalogInstallStatus, ConflictPolicy,
    build_catalog_rows, filter_catalog_rows, list_alternate_sources, resolve_agent,
    resolve_effective_agents,
};
use std::collections::BTreeMap;

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

fn registry_entry(id: &str, name: &str, version: &str, description: &str) -> RegistryCatalogEntry {
    RegistryCatalogEntry {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        version: version.into(),
    }
}

fn registry_manifest(id: &str, name: &str, version: &str, description: &str) -> RegistryManifest {
    RegistryManifest {
        id: id.into(),
        name: name.into(),
        version: version.into(),
        description: description.into(),
        repository: None,
        authors: Vec::new(),
        license: None,
        icon: None,
        distribution: RegistryDistribution {
            npx: Some(RegistryPackageDistribution {
                package: format!("{id}@{version}"),
                args: vec!["--acp".into()],
                env: BTreeMap::new(),
            }),
            uvx: None,
            binary: BTreeMap::new(),
        },
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

#[test]
fn installed_filter_keeps_managed_registry_agents() {
    let rows = resolve_effective_agents(
        vec![managed_candidate(
            "official",
            agent_spec("codex-acp", "Codex CLI", "codex-acp"),
            "0.10.0",
        )],
        ConflictPolicy::LocalWins,
    );
    let cached = CachedRegistryData {
        index: vec![registry_entry(
            "codex-acp",
            "Codex CLI",
            "0.10.0",
            "ACP adapter for OpenAI",
        )],
        meta: None,
        manifests: BTreeMap::from([(
            "codex-acp".into(),
            registry_manifest("codex-acp", "Codex CLI", "0.10.0", "ACP adapter for OpenAI"),
        )]),
    };

    let catalog = build_catalog_rows(Some(&cached), &rows);
    let filtered = filter_catalog_rows(&catalog, CatalogFilter::Installed, "");

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].acp_id, "codex-acp");
    assert_eq!(filtered[0].install_status, CatalogInstallStatus::Installed);
}

#[test]
fn not_installed_filter_keeps_registry_only_agents() {
    let cached = CachedRegistryData {
        index: vec![registry_entry(
            "amp-acp",
            "Amp",
            "0.7.0",
            "ACP wrapper for Amp",
        )],
        meta: None,
        manifests: BTreeMap::from([(
            "amp-acp".into(),
            registry_manifest("amp-acp", "Amp", "0.7.0", "ACP wrapper for Amp"),
        )]),
    };

    let catalog = build_catalog_rows(Some(&cached), &[]);
    let filtered = filter_catalog_rows(&catalog, CatalogFilter::NotInstalled, "");

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].acp_id, "amp-acp");
    assert_eq!(
        filtered[0].install_status,
        CatalogInstallStatus::NotInstalled
    );
}

#[test]
fn update_available_filter_keeps_registry_agents_with_newer_versions() {
    let rows = resolve_effective_agents(
        vec![AgentCandidate {
            agent_key: AgentKey {
                source_type: AgentSourceKind::Registry,
                source_id: "official".into(),
                acp_id: "codex-acp".into(),
            },
            spec: agent_spec("codex-acp", "Codex CLI", "codex-acp"),
            managed_state: Some(ManagedAgentState {
                id: "codex-acp".into(),
                installed_version: Some("0.9.0".into()),
                latest_registry_version: Some("0.10.0".into()),
                update_available: true,
                ..Default::default()
            }),
        }],
        ConflictPolicy::LocalWins,
    );
    let cached = CachedRegistryData {
        index: vec![registry_entry(
            "codex-acp",
            "Codex CLI",
            "0.10.0",
            "ACP adapter for OpenAI",
        )],
        meta: None,
        manifests: BTreeMap::from([(
            "codex-acp".into(),
            registry_manifest("codex-acp", "Codex CLI", "0.10.0", "ACP adapter for OpenAI"),
        )]),
    };

    let catalog = build_catalog_rows(Some(&cached), &rows);
    let filtered = filter_catalog_rows(&catalog, CatalogFilter::UpdateAvailable, "");

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].acp_id, "codex-acp");
    assert_eq!(
        filtered[0].install_status,
        CatalogInstallStatus::UpdateAvailable
    );
}

#[test]
fn catalog_rows_keep_other_sources_for_disclosure() {
    let rows = resolve_effective_agents(
        vec![
            managed_candidate(
                "official",
                agent_spec("codex-acp", "Codex CLI", "codex-acp"),
                "0.10.0",
            ),
            candidate(
                AgentSourceKind::WorkspaceCustom,
                "workspace",
                agent_spec("codex-acp", "Codex Workspace", "codex-local"),
            ),
        ],
        ConflictPolicy::LocalWins,
    );
    let cached = CachedRegistryData {
        index: vec![registry_entry(
            "codex-acp",
            "Codex CLI",
            "0.10.0",
            "ACP adapter for OpenAI",
        )],
        meta: None,
        manifests: BTreeMap::from([(
            "codex-acp".into(),
            registry_manifest("codex-acp", "Codex CLI", "0.10.0", "ACP adapter for OpenAI"),
        )]),
    };

    let catalog = build_catalog_rows(Some(&cached), &rows);

    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].acp_id, "codex-acp");
    assert_eq!(catalog[0].other_sources.len(), 1);
    assert_eq!(
        catalog[0].other_sources[0].agent_key.source_type,
        AgentSourceKind::Registry
    );
}
