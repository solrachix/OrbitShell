use crate::acp::resolve::AgentKey;
use crate::acp::storage;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const PREFERENCES_FILE: &str = "acp-runtime-preferences.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RuntimePreferences {
    entries: Vec<RuntimePreferenceEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RuntimePreferenceEntry {
    agent_key: AgentKey,
    default_model: Option<String>,
}

impl RuntimePreferences {
    pub fn load() -> Result<Self> {
        let path = Self::file_path()?;
        match storage::load_optional_json_file(&path)? {
            Some(data) => Ok(data),
            None => Ok(Self::default()),
        }
    }

    pub fn default_model_for(&self, agent_key: &AgentKey) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| &entry.agent_key == agent_key)
            .and_then(|entry| entry.default_model.clone())
    }

    pub fn set_default_model<T: Into<Option<String>>>(
        &mut self,
        agent_key: AgentKey,
        model: T,
    ) -> Result<()> {
        let model = model.into();
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.agent_key == agent_key)
        {
            entry.default_model = model.clone();
        } else if model.is_some() {
            self.entries.push(RuntimePreferenceEntry {
                agent_key,
                default_model: model.clone(),
            });
        }
        self.save()
    }

    pub fn clear_default_model(&mut self, agent_key: &AgentKey) -> Result<()> {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.agent_key == agent_key)
        {
            entry.default_model = None;
        }
        self.save()
    }

    pub fn ensure_default_model_valid(
        &mut self,
        agent_key: &AgentKey,
        available_ids: &[String],
    ) -> Result<()> {
        if available_ids.is_empty() {
            return Ok(());
        }

        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.agent_key == agent_key)
        {
            if let Some(default_model) = &entry.default_model {
                if !available_ids.iter().any(|id| id == default_model) {
                    entry.default_model = None;
                    self.save()?;
                }
            }
        }

        Ok(())
    }

    fn save(&self) -> Result<()> {
        let path = Self::file_path()?;
        storage::save_json_file(&path, self)
    }

    fn file_path() -> Result<PathBuf> {
        Ok(storage::app_root()?.join(PREFERENCES_FILE))
    }
}
