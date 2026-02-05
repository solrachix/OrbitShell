use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub last_opened: i64,
}

pub fn load_recent() -> Vec<RecentEntry> {
    let Some(path) = recent_file() else {
        return Vec::new();
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

pub fn add_recent(path: PathBuf) -> Vec<RecentEntry> {
    let mut items = load_recent();
    let now = chrono::Utc::now().timestamp();
    if let Some(existing) = items.iter_mut().find(|e| e.path == path) {
        existing.last_opened = now;
    } else {
        items.push(RecentEntry {
            path: path.clone(),
            last_opened: now,
        });
    }
    items.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
    items.truncate(20);
    let _ = save_recent(&items);
    items
}

pub fn save_recent(items: &[RecentEntry]) -> std::io::Result<()> {
    let Some(path) = recent_file() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(items).unwrap_or_default();
    std::fs::write(path, data)?;
    Ok(())
}

fn recent_file() -> Option<PathBuf> {
    let base = data_dir()?;
    Some(base.join("orbitshell").join("recent.json"))
}

fn data_dir() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(PathBuf::from(appdata));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return Some(PathBuf::from(xdg));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Some(PathBuf::from(home).join(".local").join("share"));
    }
    None
}
