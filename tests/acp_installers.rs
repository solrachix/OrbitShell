use orbitshell::acp::install::binary::{BinaryInstallSpec, install_binary_from_file};
use orbitshell::acp::install::runner::{
    LaunchCommand, build_npx_launch, build_uvx_launch, choose_npx_launch, npx_launch_candidates,
    remove_launch_wrapper, write_launch_wrapper,
};
use orbitshell::acp::install::state::{ManagedAgentState, ManagedInstalledVersion};
use sha2::{Digest, Sha256};
use std::io::Write;

#[test]
fn npx_wrapper_pins_package_version() {
    let launch = build_npx_launch("@zed-industries/codex-acp", "0.10.0");
    assert!(
        launch
            .args
            .iter()
            .any(|arg| arg.contains("@zed-industries/codex-acp@0.10.0"))
    );
}

#[test]
fn npx_launch_candidates_are_ordered_with_windows_fallbacks_first() {
    let candidates = npx_launch_candidates("@zed-industries/codex-acp@0.10.0", &[]);
    if cfg!(windows) {
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].command, "npm.cmd");
        assert_eq!(candidates[1].command, "npx.cmd");
        assert_eq!(candidates[2].command, "npx");
    } else {
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].command, "npx");
    }
}

#[test]
fn choose_npx_launch_picks_first_working_candidate() {
    let selected = choose_npx_launch("@zed-industries/codex-acp@0.10.0", &[], |candidate| {
        Ok(candidate.command == if cfg!(windows) { "npx.cmd" } else { "npx" })
    })
    .expect("select launch");

    if cfg!(windows) {
        assert_eq!(selected.command, "npx.cmd");
    } else {
        assert_eq!(selected.command, "npx");
    }
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
        launcher_command: Some("npm.cmd".into()),
        launcher_args: vec!["exec".into()],
    });
    state.record_installed_version(ManagedInstalledVersion {
        version: "1.0.0".into(),
        install_root: "C:/tmp/1.0.0".into(),
        resolved_command: "npx".into(),
        resolved_args: vec!["pkg@1.0.0".into()],
        launcher_command: Some("npx.cmd".into()),
        launcher_args: vec!["-y".into()],
    });
    state.set_active_version("1.0.0");

    let active = state.active_install().expect("active install");
    assert_eq!(active.version, "1.0.0");
    assert_eq!(active.resolved_command, "npx");
    assert_eq!(active.launcher_command.as_deref(), Some("npx.cmd"));
}

#[test]
fn binary_checksum_mismatch_aborts_activation() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let artifact_path = temp.path().join("codex-acp.exe");
    std::fs::write(&artifact_path, b"binary-payload").expect("write binary payload");
    let mut state = ManagedAgentState {
        id: "codex-acp".into(),
        ..Default::default()
    };

    let result = install_binary_from_file(
        &artifact_path,
        temp.path().join("installs").as_path(),
        &BinaryInstallSpec {
            version: "1.0.0".into(),
            sha256: "deadbeef".into(),
            executable_path: "codex-acp.exe".into(),
            archive_kind: None,
            args: Vec::new(),
        },
        &mut state,
    );

    assert!(result.is_err());
    assert!(result.err().unwrap().to_string().contains("checksum"));
    assert!(state.active_version.is_none());
}

#[test]
fn verified_binary_artifact_becomes_active_version() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let artifact_path = temp.path().join("codex-acp.exe");
    let payload = b"binary-payload";
    std::fs::write(&artifact_path, payload).expect("write binary payload");
    let sha256 = format!("{:x}", Sha256::digest(payload));
    let mut state = ManagedAgentState {
        id: "codex-acp".into(),
        ..Default::default()
    };

    let executable = install_binary_from_file(
        &artifact_path,
        temp.path().join("installs").as_path(),
        &BinaryInstallSpec {
            version: "1.0.0".into(),
            sha256,
            executable_path: "codex-acp.exe".into(),
            archive_kind: None,
            args: Vec::new(),
        },
        &mut state,
    )
    .expect("install verified binary");

    assert!(executable.exists());
    assert_eq!(state.active_version.as_deref(), Some("1.0.0"));
    assert_eq!(
        state
            .active_install()
            .map(|install| install.version.as_str()),
        Some("1.0.0")
    );
}

#[test]
fn verified_tar_bz2_binary_artifact_becomes_active_version() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let artifact_path = temp.path().join("goose.tar.bz2");
    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        let payload = b"#!/usr/bin/env sh\necho goose\n";
        let mut header = tar::Header::new_gnu();
        header.set_path("goose").expect("set tar path");
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append(&header, &payload[..])
            .expect("append tar payload");
        builder.finish().expect("finish tar");
    }
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    encoder.write_all(&tar_bytes).expect("compress tar archive");
    let archive_bytes = encoder.finish().expect("finish bzip2 archive");
    std::fs::write(&artifact_path, &archive_bytes).expect("write tar.bz2 payload");
    let sha256 = format!("{:x}", Sha256::digest(&archive_bytes));
    let mut state = ManagedAgentState {
        id: "goose".into(),
        ..Default::default()
    };

    let executable = install_binary_from_file(
        &artifact_path,
        temp.path().join("installs").as_path(),
        &BinaryInstallSpec {
            version: "1.27.2".into(),
            sha256,
            executable_path: "goose".into(),
            archive_kind: Some("tar.bz2".into()),
            args: vec!["acp".into()],
        },
        &mut state,
    )
    .expect("install verified tar.bz2 binary");

    assert!(executable.exists());
    assert_eq!(state.active_version.as_deref(), Some("1.27.2"));
    assert_eq!(
        state
            .active_install()
            .map(|install| install.resolved_args.clone()),
        Some(vec!["acp".into()])
    );
}

#[test]
fn unsupported_or_unverifiable_binary_metadata_is_rejected() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let artifact_path = temp.path().join("codex-acp.exe");
    std::fs::write(&artifact_path, b"binary-payload").expect("write binary payload");
    let mut state = ManagedAgentState {
        id: "codex-acp".into(),
        ..Default::default()
    };

    let result = install_binary_from_file(
        &artifact_path,
        temp.path().join("installs").as_path(),
        &BinaryInstallSpec {
            version: "1.0.0".into(),
            sha256: "".into(),
            executable_path: "".into(),
            archive_kind: Some("rar".into()),
            args: Vec::new(),
        },
        &mut state,
    );

    assert!(result.is_err());
    assert!(state.active_version.is_none());
}
