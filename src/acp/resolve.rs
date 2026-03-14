use crate::acp::install::state::ManagedAgentState;
use crate::acp::install::state::ManagedAgentsStateFile;
use crate::acp::manager::AgentRegistry;
use crate::acp::manager::AgentSpec;
use crate::acp::registry::cache;
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
            env_keys: manifest.env_keys.clone(),
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
