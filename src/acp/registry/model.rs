use serde::{Deserialize, Serialize};

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
pub struct RegistryDistribution {
    pub kind: String,
    pub package: Option<String>,
    pub executable: Option<String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub archive_kind: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env_keys: Vec<String>,
    pub distribution: RegistryDistribution,
}
