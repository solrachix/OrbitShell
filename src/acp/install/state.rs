use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedInstalledVersion {
    pub version: String,
    pub install_root: PathBuf,
    pub resolved_command: String,
    #[serde(default)]
    pub resolved_args: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
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
    #[serde(default)]
    pub installed_versions: Vec<ManagedInstalledVersion>,
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

impl ManagedAgentState {
    pub fn record_installed_version(&mut self, install: ManagedInstalledVersion) {
        if let Some(existing) = self
            .installed_versions
            .iter_mut()
            .find(|existing| existing.version == install.version)
        {
            *existing = install.clone();
        } else {
            self.installed_versions.push(install.clone());
        }

        self.installed_version = Some(install.version.clone());
        if self.active_version.is_none() {
            self.active_version = Some(install.version);
        }
    }

    pub fn set_active_version(&mut self, version: &str) {
        self.active_version = Some(version.to_string());
        self.installed_version = Some(version.to_string());
    }

    pub fn active_install(&self) -> Option<&ManagedInstalledVersion> {
        let active_version = self.active_version.as_deref()?;
        self.installed_versions
            .iter()
            .find(|install| install.version == active_version)
    }
}

impl ManagedAgentsStateFile {
    pub fn find_mut(&mut self, id: &str) -> Option<&mut ManagedAgentState> {
        self.agents.iter_mut().find(|agent| agent.id == id)
    }
}
