use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::acp::storage;
use crate::mcp::probe::McpProbeResult;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    pub url: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub enabled: bool,
    pub last_tested_at: Option<i64>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlobalMcpConfig {
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

impl GlobalMcpConfig {
    pub fn default_path() -> Result<PathBuf> {
        Ok(storage::app_root()?.join("mcp-servers.json"))
    }

    pub fn load_default() -> Result<Self> {
        Self::load_from_path(&Self::default_path()?)
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        Ok(storage::load_optional_json_file(path)?.unwrap_or_default())
    }

    pub fn save_default(&self) -> Result<()> {
        self.save_to_path(&Self::default_path()?)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        storage::save_json_file(path, self)
    }

    pub fn enabled_servers(&self) -> impl Iterator<Item = &McpServerConfig> {
        self.servers.iter().filter(|server| server.enabled)
    }

    pub fn upsert_server(&mut self, server: McpServerConfig) {
        if let Some(existing) = self.servers.iter_mut().find(|item| item.id == server.id) {
            *existing = server;
        } else {
            self.servers.push(server);
        }
    }

    pub fn remove_server(&mut self, id: &str) -> bool {
        let original_len = self.servers.len();
        self.servers.retain(|server| server.id != id);
        self.servers.len() != original_len
    }

    pub fn apply_probe_result(&mut self, id: &str, result: &McpProbeResult) -> bool {
        let Some(server) = self.servers.iter_mut().find(|server| server.id == id) else {
            return false;
        };

        server.last_tested_at = Some(result.tested_at);
        server.last_status = Some(result.status.clone());
        server.last_error = result.error.clone();
        true
    }
}
