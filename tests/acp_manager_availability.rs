use orbitshell::acp::manager::AgentSpec;
use std::collections::BTreeMap;
use std::fs;
use tempfile::tempdir;

#[test]
fn absolute_or_pathlike_command_is_available_when_file_exists() {
    let temp = tempdir().expect("temp dir");
    let command_path = temp.path().join(if cfg!(windows) {
        "agent.cmd"
    } else {
        "agent.sh"
    });
    fs::write(&command_path, "echo ok").expect("write command file");

    let agent = AgentSpec {
        id: "test-agent".to_string(),
        name: "Test Agent".to_string(),
        command: command_path.to_string_lossy().to_string(),
        args: Vec::new(),
        fixed_env: BTreeMap::new(),
        env_keys: Vec::new(),
        install: None,
        auth: None,
    };

    assert!(agent.is_available());
}

#[test]
fn absolute_or_pathlike_command_is_unavailable_when_file_missing() {
    let temp = tempdir().expect("temp dir");
    let command_path = temp.path().join(if cfg!(windows) {
        "missing.cmd"
    } else {
        "missing.sh"
    });

    let agent = AgentSpec {
        id: "test-agent".to_string(),
        name: "Test Agent".to_string(),
        command: command_path.to_string_lossy().to_string(),
        args: Vec::new(),
        fixed_env: BTreeMap::new(),
        env_keys: Vec::new(),
        install: None,
        auth: None,
    };

    assert!(!agent.is_available());
}
