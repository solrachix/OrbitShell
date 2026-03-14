use anyhow::{Result, anyhow};
use orbitshell::acp::install::state::{ManagedAgentState, ManagedAgentsStateFile};
use orbitshell::acp::registry::fetch::{
    FetchResponse, RegistryFetchClient, RegistrySnapshot, detect_available_updates,
    load_cached_registry, load_then_refresh, parse_registry_snapshot_json,
};
use orbitshell::acp::registry::model::{
    RegistryCacheMeta, RegistryCatalogEntry, RegistryDistribution, RegistryManifest,
    RegistryPackageDistribution,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn temp_app_root() -> PathBuf {
    let unique = format!(
        "orbitshell-registry-cache-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos()
    );
    orbitshell::acp::storage::app_root_from(std::env::temp_dir().join(unique))
}

#[derive(Clone)]
enum FakeFetchMode {
    Snapshot(RegistrySnapshot),
    Fail(String),
}

#[derive(Clone)]
struct FakeFetchClient {
    mode: FakeFetchMode,
}

impl RegistryFetchClient for FakeFetchClient {
    fn fetch_snapshot(&self, _etag: Option<&str>) -> Result<FetchResponse> {
        match &self.mode {
            FakeFetchMode::Snapshot(snapshot) => Ok(FetchResponse::Snapshot(snapshot.clone())),
            FakeFetchMode::Fail(message) => Err(anyhow!(message.clone())),
        }
    }
}

fn manifest_with_version(version: &str) -> RegistryManifest {
    RegistryManifest {
        id: "codex-acp".into(),
        name: "Codex CLI".into(),
        description: "OpenAI ACP adapter".into(),
        version: version.into(),
        repository: Some("https://github.com/zed-industries/codex-acp".into()),
        authors: vec!["OpenAI".into()],
        license: Some("Apache-2.0".into()),
        icon: Some("https://cdn.agentclientprotocol.com/registry/v1/latest/codex-acp.svg".into()),
        distribution: RegistryDistribution {
            npx: Some(RegistryPackageDistribution {
                package: format!("@zed-industries/codex-acp@{version}"),
                args: vec!["--acp".into()],
                env: BTreeMap::new(),
            }),
            uvx: None,
            binary: BTreeMap::new(),
        },
    }
}

#[test]
fn registry_cache_round_trips_index_meta_and_manifest() {
    let app_root = temp_app_root();
    let index = vec![RegistryCatalogEntry {
        id: "codex-acp".into(),
        name: "Codex CLI".into(),
        description: "OpenAI ACP adapter".into(),
        version: "0.10.0".into(),
    }];
    let meta = RegistryCacheMeta {
        last_fetch: Some(1_731_231_231),
        etag: Some("W/\"etag-1\"".into()),
        ttl_seconds: 3600,
    };
    let manifest = manifest_with_version("0.10.0");

    orbitshell::acp::registry::cache::save_registry_index(&app_root, &index)
        .expect("save registry index");
    orbitshell::acp::registry::cache::save_registry_meta(&app_root, &meta)
        .expect("save registry meta");
    orbitshell::acp::registry::cache::save_registry_manifest(&app_root, &manifest)
        .expect("save registry manifest");

    let loaded_index = orbitshell::acp::registry::cache::load_registry_index(&app_root)
        .expect("load registry index")
        .expect("registry index should exist");
    let loaded_meta = orbitshell::acp::registry::cache::load_registry_meta(&app_root)
        .expect("load registry meta")
        .expect("registry meta should exist");
    let loaded_manifest =
        orbitshell::acp::registry::cache::load_registry_manifest(&app_root, "codex-acp")
            .expect("load registry manifest")
            .expect("registry manifest should exist");

    assert_eq!(loaded_index, index);
    assert_eq!(loaded_meta.etag.as_deref(), Some("W/\"etag-1\""));
    assert_eq!(loaded_meta.ttl_seconds, 3600);
    assert_eq!(loaded_manifest, manifest);
    assert!(
        orbitshell::acp::registry::cache::registry_index_path(&app_root).exists(),
        "expected registry-index.json to exist"
    );
    assert!(
        orbitshell::acp::registry::cache::registry_meta_path(&app_root).exists(),
        "expected registry-meta.json to exist"
    );
    assert!(
        orbitshell::acp::registry::cache::registry_manifest_path(&app_root, "codex-acp").exists(),
        "expected manifest cache file to exist"
    );

    std::fs::remove_dir_all(app_root.parent().expect("app root parent")).expect("cleanup temp dir");
}

#[test]
fn load_then_refresh_replaces_cached_registry_with_remote_snapshot() {
    let app_root = temp_app_root();
    let cached_index = vec![RegistryCatalogEntry {
        id: "codex-acp".into(),
        name: "Codex CLI".into(),
        description: "cached".into(),
        version: "0.9.0".into(),
    }];
    orbitshell::acp::registry::cache::save_registry_index(&app_root, &cached_index)
        .expect("save cached index");
    orbitshell::acp::registry::cache::save_registry_meta(
        &app_root,
        &RegistryCacheMeta {
            last_fetch: Some(1),
            etag: Some("etag-cached".into()),
            ttl_seconds: 60,
        },
    )
    .expect("save cached meta");
    orbitshell::acp::registry::cache::save_registry_manifest(
        &app_root,
        &manifest_with_version("0.9.0"),
    )
    .expect("save cached manifest");

    let cached = load_cached_registry(&app_root)
        .expect("load cached registry")
        .expect("cached registry should exist");
    assert_eq!(cached.index[0].version, "0.9.0");

    let result = load_then_refresh(
        &app_root,
        &FakeFetchClient {
            mode: FakeFetchMode::Snapshot(RegistrySnapshot {
                index: vec![RegistryCatalogEntry {
                    id: "codex-acp".into(),
                    name: "Codex CLI".into(),
                    description: "remote".into(),
                    version: "1.0.0".into(),
                }],
                manifests: vec![manifest_with_version("1.0.0")],
                etag: Some("etag-remote".into()),
                fetched_at: 999,
                ttl_seconds: 3600,
            }),
        },
        None,
    )
    .expect("refresh registry");

    assert!(!result.used_cache);
    assert_eq!(result.data.index[0].version, "1.0.0");
    assert_eq!(
        result
            .data
            .meta
            .as_ref()
            .and_then(|meta| meta.etag.as_deref()),
        Some("etag-remote")
    );

    std::fs::remove_dir_all(app_root.parent().expect("app root parent")).expect("cleanup temp dir");
}

#[test]
fn load_then_refresh_keeps_cached_registry_when_refresh_fails() {
    let app_root = temp_app_root();
    let cached_index = vec![RegistryCatalogEntry {
        id: "codex-acp".into(),
        name: "Codex CLI".into(),
        description: "cached".into(),
        version: "0.9.0".into(),
    }];
    orbitshell::acp::registry::cache::save_registry_index(&app_root, &cached_index)
        .expect("save cached index");
    orbitshell::acp::registry::cache::save_registry_meta(
        &app_root,
        &RegistryCacheMeta {
            last_fetch: Some(5),
            etag: Some("etag-cached".into()),
            ttl_seconds: 60,
        },
    )
    .expect("save cached meta");
    orbitshell::acp::registry::cache::save_registry_manifest(
        &app_root,
        &manifest_with_version("0.9.0"),
    )
    .expect("save cached manifest");

    let result = load_then_refresh(
        &app_root,
        &FakeFetchClient {
            mode: FakeFetchMode::Fail("network down".into()),
        },
        None,
    )
    .expect("fall back to cache");

    assert!(result.used_cache);
    assert_eq!(result.data.index[0].version, "0.9.0");
    assert!(
        result
            .refresh_error
            .as_deref()
            .unwrap_or_default()
            .contains("network down")
    );

    std::fs::remove_dir_all(app_root.parent().expect("app root parent")).expect("cleanup temp dir");
}

#[test]
fn refresh_marks_update_available_when_registry_version_is_newer() {
    let mut managed = ManagedAgentsStateFile {
        agents: vec![ManagedAgentState {
            id: "codex-acp".into(),
            installed_version: Some("0.9.0".into()),
            ..Default::default()
        }],
    };

    detect_available_updates(
        &mut managed,
        &[RegistryCatalogEntry {
            id: "codex-acp".into(),
            name: "Codex CLI".into(),
            description: "remote".into(),
            version: "1.0.0".into(),
        }],
        Some(1234),
    );

    let state = &managed.agents[0];
    assert_eq!(state.latest_registry_version.as_deref(), Some("1.0.0"));
    assert_eq!(state.last_checked_at, Some(1234));
    assert!(state.update_available);
}

#[test]
fn parse_registry_snapshot_json_supports_official_agents_array_shape() {
    let raw = r#"
    {
      "version": "1.0.0",
      "agents": [
        {
          "id": "codex-acp",
          "name": "Codex CLI",
          "version": "0.10.0",
          "description": "ACP adapter for OpenAI's coding assistant",
          "repository": "https://github.com/zed-industries/codex-acp",
          "authors": ["OpenAI"],
          "license": "Apache-2.0",
          "icon": "https://cdn.agentclientprotocol.com/registry/v1/latest/codex-acp.svg",
          "distribution": {
            "npx": {
              "package": "@zed-industries/codex-acp@0.10.0",
              "args": ["--acp"]
            }
          }
        }
      ]
    }
    "#;

    let snapshot = parse_registry_snapshot_json(raw, Some("etag-1".into()), 99, 3600)
        .expect("parse official registry payload");

    assert_eq!(snapshot.index.len(), 1);
    assert_eq!(snapshot.manifests.len(), 1);
    assert_eq!(snapshot.index[0].id, "codex-acp");
    assert_eq!(snapshot.index[0].version, "0.10.0");
    assert_eq!(
        snapshot.manifests[0]
            .distribution
            .npx
            .as_ref()
            .map(|dist| dist.package.as_str()),
        Some("@zed-industries/codex-acp@0.10.0")
    );
}
