use crate::acp::install::state::{ManagedAgentState, ManagedAgentsStateFile};
use crate::acp::manager::{AgentRegistry, AgentSpec};
use crate::acp::registry::cache;
use crate::acp::registry::fetch::CachedRegistryData;
use crate::acp::registry::model::{
    RegistryBinaryDistribution, RegistryCatalogEntry, RegistryManifest,
};
use crate::acp::storage;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    LocalWins,
    RegistryWins,
    ShowBoth,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentSourceKind {
    Registry,
    GlobalCustom,
    WorkspaceCustom,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentKey {
    pub source_type: AgentSourceKind,
    pub source_id: String,
    pub acp_id: String,
}

#[derive(Clone, Debug)]
pub struct AgentCandidate {
    pub agent_key: AgentKey,
    pub spec: AgentSpec,
    pub managed_state: Option<ManagedAgentState>,
}

#[derive(Clone, Debug)]
pub struct EffectiveAgentRow {
    pub agent_key: AgentKey,
    pub acp_id: String,
    pub name: String,
    pub source_type: AgentSourceKind,
    pub source_id: String,
    pub spec: AgentSpec,
    pub managed_state: Option<ManagedAgentState>,
    pub is_selected: bool,
    pub is_alternate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogFilter {
    All,
    Installed,
    NotInstalled,
    UpdateAvailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogInstallStatus {
    Installed,
    NotInstalled,
    UpdateAvailable,
}

#[derive(Clone, Debug)]
pub struct CatalogAgentRow {
    pub acp_id: String,
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub registry_entry: Option<RegistryCatalogEntry>,
    pub registry_manifest: Option<RegistryManifest>,
    pub selected_source: Option<EffectiveAgentRow>,
    pub other_sources: Vec<EffectiveAgentRow>,
    pub install_status: CatalogInstallStatus,
}

pub fn resolve_effective_agents(
    candidates: Vec<AgentCandidate>,
    policy: ConflictPolicy,
) -> Vec<EffectiveAgentRow> {
    let mut grouped: BTreeMap<String, Vec<AgentCandidate>> = BTreeMap::new();
    for candidate in candidates {
        grouped
            .entry(candidate.spec.id.clone())
            .or_default()
            .push(candidate);
    }

    let mut rows = Vec::new();
    for (_acp_id, mut group) in grouped {
        group.sort_by(|left, right| {
            candidate_rank(left.agent_key.source_type, policy)
                .cmp(&candidate_rank(right.agent_key.source_type, policy))
                .then_with(|| left.agent_key.source_id.cmp(&right.agent_key.source_id))
                .then_with(|| left.spec.name.cmp(&right.spec.name))
        });

        for (index, candidate) in group.into_iter().enumerate() {
            let is_selected = matches!(policy, ConflictPolicy::ShowBoth) || index == 0;
            let is_alternate = !matches!(policy, ConflictPolicy::ShowBoth) && index > 0;
            rows.push(EffectiveAgentRow {
                acp_id: candidate.spec.id.clone(),
                name: candidate.spec.name.clone(),
                source_type: candidate.agent_key.source_type,
                source_id: candidate.agent_key.source_id.clone(),
                spec: candidate.spec,
                agent_key: candidate.agent_key,
                managed_state: candidate.managed_state,
                is_selected,
                is_alternate,
            });
        }
    }

    rows
}

pub fn build_catalog_rows(
    cached: Option<&CachedRegistryData>,
    effective_rows: &[EffectiveAgentRow],
) -> Vec<CatalogAgentRow> {
    let mut selected_by_id = BTreeMap::new();
    let mut alternates_by_id: BTreeMap<String, Vec<EffectiveAgentRow>> = BTreeMap::new();
    let mut fallback_by_id = BTreeMap::new();

    for row in effective_rows {
        fallback_by_id
            .entry(row.acp_id.clone())
            .or_insert_with(|| row.clone());

        if row.is_selected {
            selected_by_id.insert(row.acp_id.clone(), row.clone());
        } else {
            alternates_by_id
                .entry(row.acp_id.clone())
                .or_default()
                .push(row.clone());
        }
    }

    let mut rows = Vec::new();
    let mut seen = BTreeMap::<String, ()>::new();
    if let Some(cached) = cached {
        for entry in &cached.index {
            let selected = selected_by_id
                .get(&entry.id)
                .cloned()
                .or_else(|| fallback_by_id.get(&entry.id).cloned());
            let install_status = catalog_status(selected.as_ref());
            rows.push(CatalogAgentRow {
                acp_id: entry.id.clone(),
                name: entry.name.clone(),
                description: entry.description.clone(),
                version: Some(entry.version.clone()),
                registry_entry: Some(entry.clone()),
                registry_manifest: cached.manifests.get(&entry.id).cloned(),
                selected_source: selected,
                other_sources: alternates_by_id.remove(&entry.id).unwrap_or_default(),
                install_status,
            });
            seen.insert(entry.id.clone(), ());
        }
    }

    for (acp_id, selected) in selected_by_id {
        if seen.contains_key(&acp_id) {
            continue;
        }
        let install_status = catalog_status(Some(&selected));
        rows.push(CatalogAgentRow {
            acp_id: acp_id.clone(),
            name: selected.name.clone(),
            description: String::new(),
            version: selected
                .managed_state
                .as_ref()
                .and_then(|state| state.installed_version.clone()),
            registry_entry: None,
            registry_manifest: None,
            selected_source: Some(selected),
            other_sources: alternates_by_id.remove(&acp_id).unwrap_or_default(),
            install_status,
        });
    }

    rows.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.acp_id.cmp(&right.acp_id))
    });
    rows
}

pub fn filter_catalog_rows(
    rows: &[CatalogAgentRow],
    filter: CatalogFilter,
    query: &str,
) -> Vec<CatalogAgentRow> {
    let query = query.trim().to_ascii_lowercase();
    rows.iter()
        .filter(|row| matches_catalog_filter(row, filter))
        .filter(|row| {
            if query.is_empty() {
                return true;
            }
            row.acp_id.to_ascii_lowercase().contains(&query)
                || row.name.to_ascii_lowercase().contains(&query)
                || row.description.to_ascii_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

pub fn list_alternate_sources(rows: &[EffectiveAgentRow], acp_id: &str) -> Vec<EffectiveAgentRow> {
    rows.iter()
        .filter(|row| row.acp_id == acp_id && row.is_alternate)
        .cloned()
        .collect()
}

pub fn resolve_agent(rows: &[EffectiveAgentRow], agent_key: &AgentKey) -> Option<AgentSpec> {
    rows.iter()
        .find(|row| row.agent_key == *agent_key)
        .map(|row| row.spec.clone())
}

pub fn load_effective_agent_rows(policy: ConflictPolicy) -> Result<Vec<EffectiveAgentRow>> {
    let mut candidates = load_managed_candidates()?;
    candidates.extend(AgentRegistry::load_global_custom_candidates()?);
    candidates.extend(AgentRegistry::load_workspace_custom_candidates()?);
    Ok(resolve_effective_agents(candidates, policy))
}

fn load_managed_candidates() -> Result<Vec<AgentCandidate>> {
    let app_root = storage::app_root()?;
    let managed = ManagedAgentsStateFile::load_default()?;
    let mut candidates = Vec::new();

    for state in managed.agents {
        let Some(active_install) = state.active_install().cloned() else {
            continue;
        };
        let Some(manifest) = cache::load_registry_manifest(&app_root, &state.id)? else {
            continue;
        };
        let spec = AgentSpec {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            command: active_install.resolved_command.clone(),
            args: active_install.resolved_args.clone(),
            fixed_env: managed_distribution_env(&manifest, &state),
            env_keys: Vec::new(),
            install: None,
            auth: None,
        };
        candidates.push(AgentCandidate {
            agent_key: AgentKey {
                source_type: AgentSourceKind::Registry,
                source_id: "managed".into(),
                acp_id: manifest.id,
            },
            spec,
            managed_state: Some(state),
        });
    }

    Ok(candidates)
}

fn managed_distribution_env(
    manifest: &RegistryManifest,
    state: &ManagedAgentState,
) -> BTreeMap<String, String> {
    match state.distribution_kind.as_deref() {
        Some("npx") => manifest
            .distribution
            .npx
            .as_ref()
            .map(|dist| dist.env.clone())
            .unwrap_or_default(),
        Some("uvx") => manifest
            .distribution
            .uvx
            .as_ref()
            .map(|dist| dist.env.clone())
            .unwrap_or_default(),
        Some("binary") => current_binary_distribution(manifest)
            .map(|dist| dist.env.clone())
            .unwrap_or_default(),
        _ => BTreeMap::new(),
    }
}

fn current_binary_distribution(manifest: &RegistryManifest) -> Option<&RegistryBinaryDistribution> {
    manifest.distribution.binary.get(&current_platform_key())
}

fn current_platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

fn catalog_status(selected: Option<&EffectiveAgentRow>) -> CatalogInstallStatus {
    let Some(selected) = selected else {
        return CatalogInstallStatus::NotInstalled;
    };
    if selected
        .managed_state
        .as_ref()
        .map(|state| state.update_available)
        .unwrap_or(false)
    {
        return CatalogInstallStatus::UpdateAvailable;
    }
    if selected
        .managed_state
        .as_ref()
        .and_then(|state| state.installed_version.as_ref())
        .is_some()
        || selected.spec.is_available()
    {
        CatalogInstallStatus::Installed
    } else {
        CatalogInstallStatus::NotInstalled
    }
}

fn matches_catalog_filter(row: &CatalogAgentRow, filter: CatalogFilter) -> bool {
    match filter {
        CatalogFilter::All => true,
        CatalogFilter::Installed => row.install_status == CatalogInstallStatus::Installed,
        CatalogFilter::NotInstalled => row.install_status == CatalogInstallStatus::NotInstalled,
        CatalogFilter::UpdateAvailable => {
            row.install_status == CatalogInstallStatus::UpdateAvailable
        }
    }
}

fn candidate_rank(source_type: AgentSourceKind, policy: ConflictPolicy) -> u8 {
    match policy {
        ConflictPolicy::LocalWins | ConflictPolicy::ShowBoth => match source_type {
            AgentSourceKind::WorkspaceCustom => 0,
            AgentSourceKind::GlobalCustom => 1,
            AgentSourceKind::Registry => 2,
        },
        ConflictPolicy::RegistryWins => match source_type {
            AgentSourceKind::Registry => 0,
            AgentSourceKind::WorkspaceCustom => 1,
            AgentSourceKind::GlobalCustom => 2,
        },
    }
}
