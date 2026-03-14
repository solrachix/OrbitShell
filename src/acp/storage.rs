use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs;
use std::path::{Path, PathBuf};

const APP_ROOT_DIR: &str = "orbitshell";

pub fn app_root() -> Result<PathBuf> {
    Ok(app_root_from(app_data_base_dir()?))
}

pub fn app_root_from(base: PathBuf) -> PathBuf {
    base.join(APP_ROOT_DIR)
}

pub fn registry_root(app_root: &Path) -> PathBuf {
    app_root.join("registry")
}

pub fn registry_cache_root(app_root: &Path) -> PathBuf {
    registry_root(app_root).join("cache")
}

pub fn registry_state_root(app_root: &Path) -> PathBuf {
    registry_root(app_root).join("state")
}

pub fn registry_installs_root(app_root: &Path) -> PathBuf {
    registry_root(app_root).join("installs")
}

pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }
    Ok(())
}

pub fn load_json_file<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read JSON file {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse JSON file {}", path.display()))
}

pub fn load_optional_json_file<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    load_json_file(path).map(Some)
}

pub fn save_json_file<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    ensure_parent_dir(path)?;
    let raw = serde_json::to_string_pretty(value).context("failed to serialize JSON value")?;
    fs::write(path, raw).with_context(|| format!("failed to write JSON file {}", path.display()))
}

fn app_data_base_dir() -> Result<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .context("APPDATA is not set");
    }

    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(dir));
    }

    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".config"))
        .context("HOME is not set")
}
