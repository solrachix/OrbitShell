use orbitshell::acp::registry::model::{
    RegistryCacheMeta, RegistryCatalogEntry, RegistryDistribution, RegistryManifest,
};
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
    let manifest = RegistryManifest {
        id: "codex-acp".into(),
        name: "Codex CLI".into(),
        description: "OpenAI ACP adapter".into(),
        version: "0.10.0".into(),
        command: "codex-acp".into(),
        args: vec!["--stdio".into()],
        env_keys: vec!["OPENAI_API_KEY".into()],
        distribution: RegistryDistribution {
            kind: "npx".into(),
            package: Some("@zed-industries/codex-acp".into()),
            executable: Some("codex-acp".into()),
            url: None,
            sha256: None,
            archive_kind: None,
        },
    };

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
