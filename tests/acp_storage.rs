use orbitshell::ui::Workspace;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[test]
fn library_exports_workspace_type() {
    let _ = std::any::type_name::<Workspace>();
}

#[test]
fn registry_paths_live_under_appdata() {
    let root = orbitshell::acp::storage::app_root_from(PathBuf::from("C:/tmp/appdata"));
    assert!(root.ends_with("orbitshell"));
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct StorageSmoke {
    id: String,
}

#[test]
fn storage_json_helpers_round_trip() {
    let unique = format!(
        "orbitshell-storage-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    let path = dir.join("sample.json");
    let sample = StorageSmoke { id: "alpha".into() };

    orbitshell::acp::storage::save_json_file(&path, &sample).expect("save sample JSON");
    let loaded: StorageSmoke =
        orbitshell::acp::storage::load_json_file(&path).expect("load sample JSON");

    assert_eq!(loaded, sample);

    std::fs::remove_dir_all(dir).expect("remove temp storage directory");
}
