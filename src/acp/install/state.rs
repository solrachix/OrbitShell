use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedAgentState {
    pub id: String,
    pub installed_version: Option<String>,
    pub latest_registry_version: Option<String>,
    pub distribution_kind: Option<String>,
    pub install_root: Option<PathBuf>,
    pub resolved_command: Option<String>,
    #[serde(default)]
    pub resolved_args: Vec<String>,
    pub active_version: Option<String>,
    pub last_install_at: Option<i64>,
    pub last_checked_at: Option<i64>,
    pub status: Option<String>,
    #[serde(default)]
    pub auth_required: bool,
    pub install_error: Option<String>,
    #[serde(default)]
    pub update_available: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedAgentsStateFile {
    #[serde(default)]
    pub agents: Vec<ManagedAgentState>,
}
