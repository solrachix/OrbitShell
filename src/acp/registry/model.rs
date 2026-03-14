use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryCacheMeta {
    pub last_fetch: Option<i64>,
    pub etag: Option<String>,
    pub ttl_seconds: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryCatalogEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryPackageDistribution {
    pub package: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryBinaryDistribution {
    pub archive: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryDistribution {
    #[serde(default)]
    pub npx: Option<RegistryPackageDistribution>,
    #[serde(default)]
    pub uvx: Option<RegistryPackageDistribution>,
    #[serde(default)]
    pub binary: BTreeMap<String, RegistryBinaryDistribution>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub distribution: RegistryDistribution,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryInstallStrategy {
    Npx(RegistryPackageDistribution),
    Uvx(RegistryPackageDistribution),
    Binary {
        platform_key: String,
        target: RegistryBinaryDistribution,
    },
}

impl RegistryManifest {
    pub fn catalog_entry(&self) -> RegistryCatalogEntry {
        RegistryCatalogEntry {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            version: self.version.clone(),
        }
    }

    pub fn preferred_install_strategy(&self) -> Option<RegistryInstallStrategy> {
        if let Some(npx) = &self.distribution.npx {
            return Some(RegistryInstallStrategy::Npx(npx.clone()));
        }
        if let Some(uvx) = &self.distribution.uvx {
            return Some(RegistryInstallStrategy::Uvx(uvx.clone()));
        }

        let platform_key = registry_platform_key();
        self.distribution
            .binary
            .get(&platform_key)
            .cloned()
            .map(|target| RegistryInstallStrategy::Binary {
                platform_key,
                target,
            })
    }
}

pub fn registry_platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

pub fn infer_archive_kind_from_url(url: &str) -> Option<String> {
    if url.ends_with(".zip") {
        Some("zip".into())
    } else if url.ends_with(".tar.bz2") || url.ends_with(".tbz2") {
        Some("tar.bz2".into())
    } else if url.ends_with(".tar.gz") || url.ends_with(".tgz") {
        Some("tar.gz".into())
    } else if url.ends_with(".tar") {
        Some("tar".into())
    } else {
        None
    }
}
