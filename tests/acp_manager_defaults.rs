use orbitshell::acp::manager::AgentRegistry;
use std::env;
use tempfile::tempdir;

struct CurrentDirGuard {
    previous: std::path::PathBuf,
}

impl CurrentDirGuard {
    fn change_to(path: &std::path::Path) -> Self {
        let previous = env::current_dir().expect("current dir");
        env::set_current_dir(path).expect("set current dir");
        Self { previous }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        env::set_current_dir(&self.previous).expect("restore current dir");
    }
}

#[test]
fn load_default_returns_empty_registry_when_workspace_has_no_agents_file() {
    let temp = tempdir().expect("temp dir");
    let _cwd = CurrentDirGuard::change_to(temp.path());

    let registry = AgentRegistry::load_default().expect("load default registry");

    assert!(registry.agents.is_empty());
}
