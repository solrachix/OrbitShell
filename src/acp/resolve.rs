use crate::acp::install::state::ManagedAgentState;
use crate::acp::manager::AgentSpec;
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
