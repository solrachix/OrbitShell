use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentCommandSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSpec {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env_keys: Vec<String>,
    #[serde(default)]
    pub install: Option<AgentCommandSpec>,
    #[serde(default)]
    pub auth: Option<AgentCommandSpec>,
}

impl AgentSpec {
    pub fn is_available(&self) -> bool {
        let probe = if cfg!(windows) { "where" } else { "which" };
        Command::new(probe)
            .arg(&self.command)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    pub fn display_command(&self) -> String {
        if self.args.is_empty() {
            self.command.clone()
        } else {
            format!("{} {}", self.command, self.args.join(" "))
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentRegistry {
    #[serde(default)]
    pub agents: Vec<AgentSpec>,
}

impl AgentRegistry {
    pub fn load_default() -> Result<Self> {
        let path = Self::default_path();
        if !path.exists() {
            return Ok(Self {
                agents: vec![AgentSpec {
                    id: "codex".to_string(),
                    name: "Codex ACP (local)".to_string(),
                    command: "codex-acp".to_string(),
                    args: Vec::new(),
                    env_keys: vec![
                        "OPENAI_API_KEY".to_string(),
                        "CODEX_API_KEY".to_string(),
                        "OPENAI_BASE_URL".to_string(),
                        "OPENAI_ORG_ID".to_string(),
                    ],
                    install: Some(AgentCommandSpec {
                        command: "npm".to_string(),
                        args: vec![
                            "i".to_string(),
                            "-g".to_string(),
                            "@zed-industries/codex-acp".to_string(),
                        ],
                    }),
                    auth: Some(AgentCommandSpec {
                        command: "codex".to_string(),
                        args: vec!["login".to_string()],
                    }),
                }],
            });
        }
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read agent registry at {}", path.display()))?;
        let registry: Self = serde_json::from_str(&raw)
            .with_context(|| format!("invalid JSON in {}", path.display()))?;
        Ok(registry)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        fs::create_dir_all(&parent).with_context(|| {
            format!(
                "failed to create agent registry parent directory {}",
                parent.display()
            )
        })?;
        let data =
            serde_json::to_string_pretty(self).context("failed to serialize agent registry JSON")?;
        fs::write(path, data)
            .with_context(|| format!("failed to write agent registry at {}", path.display()))?;
        Ok(())
    }

    pub fn save_default(&self) -> Result<()> {
        self.save_to_path(&Self::default_path())
    }

    pub fn default_path() -> PathBuf {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("agents.json")
    }
}
