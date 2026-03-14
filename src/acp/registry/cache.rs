use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::acp::registry::model::{RegistryCacheMeta, RegistryCatalogEntry, RegistryManifest};
use crate::acp::storage;

pub fn registry_index_path(app_root: &Path) -> PathBuf {
    storage::registry_cache_root(app_root).join("registry-index.json")
}

pub fn registry_meta_path(app_root: &Path) -> PathBuf {
    storage::registry_cache_root(app_root).join("registry-meta.json")
}

pub fn registry_manifest_path(app_root: &Path, agent_id: &str) -> PathBuf {
    storage::registry_cache_root(app_root)
        .join("manifests")
        .join(format!("{agent_id}.json"))
}

pub fn load_registry_index(app_root: &Path) -> Result<Option<Vec<RegistryCatalogEntry>>> {
    storage::load_optional_json_file(&registry_index_path(app_root))
}

pub fn save_registry_index(app_root: &Path, entries: &[RegistryCatalogEntry]) -> Result<()> {
    storage::save_json_file(&registry_index_path(app_root), entries)
}

pub fn load_registry_meta(app_root: &Path) -> Result<Option<RegistryCacheMeta>> {
    storage::load_optional_json_file(&registry_meta_path(app_root))
}

pub fn save_registry_meta(app_root: &Path, meta: &RegistryCacheMeta) -> Result<()> {
    storage::save_json_file(&registry_meta_path(app_root), meta)
}

pub fn load_registry_manifest(app_root: &Path, agent_id: &str) -> Result<Option<RegistryManifest>> {
    storage::load_optional_json_file(&registry_manifest_path(app_root, agent_id))
}

pub fn save_registry_manifest(app_root: &Path, manifest: &RegistryManifest) -> Result<()> {
    storage::save_json_file(&registry_manifest_path(app_root, &manifest.id), manifest)
}
