use orbitshell::acp::install::runner::{
    LaunchCommand, build_npx_launch, build_uvx_launch, remove_launch_wrapper, write_launch_wrapper,
};
use orbitshell::acp::install::state::{ManagedAgentState, ManagedInstalledVersion};

#[test]
fn npx_wrapper_pins_package_version() {
    let launch = build_npx_launch("@zed-industries/codex-acp", "0.10.0");
    assert!(launch.command.contains("npx"));
    assert!(
        launch
            .args
            .iter()
            .any(|arg| arg.contains("@zed-industries/codex-acp@0.10.0"))
    );
}

#[test]
fn uvx_wrapper_pins_package_version() {
    let launch = build_uvx_launch("codex-acp", "0.10.0");
    assert!(launch.command.contains("uvx"));
    assert!(
        launch
            .args
            .iter()
            .any(|arg| arg.contains("codex-acp==0.10.0"))
    );
}

#[test]
fn removing_wrapper_files_does_not_touch_external_caches() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let install_root = temp
        .path()
        .join("installs")
        .join("codex-acp")
        .join("0.10.0");
    let external_cache = temp.path().join("external-cache");
    std::fs::create_dir_all(&external_cache).expect("create external cache");
    let cache_marker = external_cache.join("marker.txt");
    std::fs::write(&cache_marker, "keep me").expect("write cache marker");

    let wrapper_path = write_launch_wrapper(
        &install_root,
        "codex-acp",
        &LaunchCommand {
            command: "npx".into(),
            args: vec!["-y".into(), "@zed-industries/codex-acp@0.10.0".into()],
        },
    )
    .expect("write wrapper");
    assert!(wrapper_path.exists());

    remove_launch_wrapper(&wrapper_path).expect("remove wrapper");

    assert!(!wrapper_path.exists());
    assert!(cache_marker.exists());
}

#[test]
fn active_version_is_driven_by_recorded_installations() {
    let mut state = ManagedAgentState {
        id: "codex-acp".into(),
        ..Default::default()
    };
    state.record_installed_version(ManagedInstalledVersion {
        version: "0.9.0".into(),
        install_root: "C:/tmp/0.9.0".into(),
        resolved_command: "npx".into(),
        resolved_args: vec!["pkg@0.9.0".into()],
    });
    state.record_installed_version(ManagedInstalledVersion {
        version: "1.0.0".into(),
        install_root: "C:/tmp/1.0.0".into(),
        resolved_command: "npx".into(),
        resolved_args: vec!["pkg@1.0.0".into()],
    });
    state.set_active_version("1.0.0");

    let active = state.active_install().expect("active install");
    assert_eq!(active.version, "1.0.0");
    assert_eq!(active.resolved_command, "npx");
}
