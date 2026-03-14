use anyhow::{Context, Result, anyhow, bail};
use semver::Version;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::acp::install::state::ManagedAgentsStateFile;
use crate::acp::registry::cache;
use crate::acp::registry::model::{RegistryCacheMeta, RegistryCatalogEntry, RegistryManifest};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CachedRegistryData {
    pub index: Vec<RegistryCatalogEntry>,
    pub meta: Option<RegistryCacheMeta>,
    pub manifests: BTreeMap<String, RegistryManifest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrySnapshot {
    pub index: Vec<RegistryCatalogEntry>,
    pub manifests: Vec<RegistryManifest>,
    pub etag: Option<String>,
    pub fetched_at: i64,
    pub ttl_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchResponse {
    Snapshot(RegistrySnapshot),
    NotModified { fetched_at: i64, ttl_seconds: u64 },
}

pub trait RegistryFetchClient {
    fn fetch_snapshot(&self, etag: Option<&str>) -> Result<FetchResponse>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryRefreshResult {
    pub data: CachedRegistryData,
    pub used_cache: bool,
    pub refresh_error: Option<String>,
}

pub fn load_cached_registry(app_root: &Path) -> Result<Option<CachedRegistryData>> {
    let Some(index) = cache::load_registry_index(app_root)? else {
        return Ok(None);
    };

    let meta = cache::load_registry_meta(app_root)?;
    let mut manifests = BTreeMap::new();
    for entry in &index {
        if let Some(manifest) = cache::load_registry_manifest(app_root, &entry.id)? {
            manifests.insert(entry.id.clone(), manifest);
        }
    }

    Ok(Some(CachedRegistryData {
        index,
        meta,
        manifests,
    }))
}

pub fn load_then_refresh(
    app_root: &Path,
    fetch_client: &impl RegistryFetchClient,
    managed_state: Option<&mut ManagedAgentsStateFile>,
) -> Result<RegistryRefreshResult> {
    let cached = load_cached_registry(app_root)?;
    let etag = cached
        .as_ref()
        .and_then(|data| data.meta.as_ref())
        .and_then(|meta| meta.etag.as_deref());

    match fetch_client.fetch_snapshot(etag) {
        Ok(FetchResponse::Snapshot(snapshot)) => {
            persist_snapshot(app_root, &snapshot)?;
            let data = cached_registry_from_snapshot(&snapshot);
            if let Some(managed_state) = managed_state {
                detect_available_updates(managed_state, &data.index, Some(snapshot.fetched_at));
            }
            Ok(RegistryRefreshResult {
                data,
                used_cache: false,
                refresh_error: None,
            })
        }
        Ok(FetchResponse::NotModified {
            fetched_at,
            ttl_seconds,
        }) => {
            let mut data = cached
                .ok_or_else(|| anyhow!("registry returned not modified but no cache exists"))?;
            let meta = RegistryCacheMeta {
                last_fetch: Some(fetched_at),
                etag: data.meta.as_ref().and_then(|item| item.etag.clone()),
                ttl_seconds,
            };
            cache::save_registry_meta(app_root, &meta)?;
            data.meta = Some(meta);
            if let Some(managed_state) = managed_state {
                detect_available_updates(managed_state, &data.index, Some(fetched_at));
            }
            Ok(RegistryRefreshResult {
                data,
                used_cache: true,
                refresh_error: None,
            })
        }
        Err(err) => {
            let err_text = err.to_string();
            let data = cached.ok_or_else(|| anyhow!(err_text.clone()))?;
            if let Some(managed_state) = managed_state {
                let checked_at = data.meta.as_ref().and_then(|meta| meta.last_fetch);
                detect_available_updates(managed_state, &data.index, checked_at);
            }
            Ok(RegistryRefreshResult {
                data,
                used_cache: true,
                refresh_error: Some(err_text),
            })
        }
    }
}

pub fn detect_available_updates(
    managed_state: &mut ManagedAgentsStateFile,
    index: &[RegistryCatalogEntry],
    checked_at: Option<i64>,
) {
    for entry in index {
        if let Some(agent) = managed_state.find_mut(&entry.id) {
            agent.latest_registry_version = Some(entry.version.clone());
            agent.last_checked_at = checked_at;
            agent.update_available = is_newer_version(
                agent.installed_version.as_deref(),
                Some(entry.version.as_str()),
            );
        }
    }
}

fn cached_registry_from_snapshot(snapshot: &RegistrySnapshot) -> CachedRegistryData {
    let mut manifests = BTreeMap::new();
    for manifest in &snapshot.manifests {
        manifests.insert(manifest.id.clone(), manifest.clone());
    }

    CachedRegistryData {
        index: snapshot.index.clone(),
        meta: Some(RegistryCacheMeta {
            last_fetch: Some(snapshot.fetched_at),
            etag: snapshot.etag.clone(),
            ttl_seconds: snapshot.ttl_seconds,
        }),
        manifests,
    }
}

fn persist_snapshot(app_root: &Path, snapshot: &RegistrySnapshot) -> Result<()> {
    cache::save_registry_index(app_root, &snapshot.index)?;
    cache::save_registry_meta(
        app_root,
        &RegistryCacheMeta {
            last_fetch: Some(snapshot.fetched_at),
            etag: snapshot.etag.clone(),
            ttl_seconds: snapshot.ttl_seconds,
        },
    )?;
    for manifest in &snapshot.manifests {
        cache::save_registry_manifest(app_root, manifest)?;
    }
    Ok(())
}

fn is_newer_version(installed: Option<&str>, latest: Option<&str>) -> bool {
    let (Some(installed), Some(latest)) = (installed, latest) else {
        return false;
    };
    match (Version::parse(installed), Version::parse(latest)) {
        (Ok(installed), Ok(latest)) => latest > installed,
        _ => latest != installed,
    }
}

#[derive(Clone, Debug)]
pub struct UreqRegistryFetchClient {
    pub index_url: String,
}

impl RegistryFetchClient for UreqRegistryFetchClient {
    fn fetch_snapshot(&self, etag: Option<&str>) -> Result<FetchResponse> {
        let agent = ureq::AgentBuilder::new().build();
        let mut request = agent.get(&self.index_url);
        if let Some(etag) = etag {
            request = request.set("If-None-Match", etag);
        }

        match request.call() {
            Ok(response) => parse_response(response),
            Err(ureq::Error::Status(304, response)) => Ok(FetchResponse::NotModified {
                fetched_at: now_timestamp()?,
                ttl_seconds: cache_ttl_from_response(&response).unwrap_or(3600),
            }),
            Err(err) => bail!("registry fetch failed: {err}"),
        }
    }
}

#[derive(Deserialize)]
struct RemoteRegistryBody {
    #[allow(dead_code)]
    version: Option<String>,
    #[serde(default)]
    index: Vec<RegistryCatalogEntry>,
    #[serde(default)]
    agents: Vec<RegistryManifest>,
    #[serde(default)]
    manifests: Vec<RegistryManifest>,
    ttl_seconds: Option<u64>,
}

fn parse_response(response: ureq::Response) -> Result<FetchResponse> {
    if response.status() == 304 {
        return Ok(FetchResponse::NotModified {
            fetched_at: now_timestamp()?,
            ttl_seconds: cache_ttl_from_response(&response).unwrap_or(3600),
        });
    }

    let ttl_seconds = cache_ttl_from_response(&response).unwrap_or(3600);
    let etag = response.header("ETag").map(ToOwned::to_owned);
    let body_text = response
        .into_string()
        .context("failed to read registry index body")?;
    Ok(FetchResponse::Snapshot(parse_registry_snapshot_json(
        &body_text,
        etag,
        now_timestamp()?,
        ttl_seconds,
    )?))
}

pub fn parse_registry_snapshot_json(
    body_text: &str,
    etag: Option<String>,
    fetched_at: i64,
    ttl_seconds: u64,
) -> Result<RegistrySnapshot> {
    let body: RemoteRegistryBody =
        serde_json::from_str(body_text).context("failed to decode registry index JSON")?;
    let manifests = if body.manifests.is_empty() {
        body.agents
    } else {
        body.manifests
    };
    let index = if body.index.is_empty() {
        manifests
            .iter()
            .map(RegistryManifest::catalog_entry)
            .collect()
    } else {
        body.index
    };

    Ok(RegistrySnapshot {
        index,
        manifests,
        etag,
        fetched_at,
        ttl_seconds: body.ttl_seconds.unwrap_or(ttl_seconds),
    })
}

fn cache_ttl_from_response(response: &ureq::Response) -> Option<u64> {
    let cache_control = response.header("Cache-Control")?;
    for part in cache_control.split(',') {
        let trimmed = part.trim();
        if let Some(value) = trimmed.strip_prefix("max-age=") {
            if let Ok(parsed) = value.parse::<u64>() {
                return Some(parsed);
            }
        }
    }
    None
}

fn now_timestamp() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before unix epoch")?;
    Ok(duration.as_secs() as i64)
}
