use anyhow::Result;
use once_cell::sync::Lazy;
use orbitshell::acp::model_discovery;
use orbitshell::acp::registry::model::{
    RegistryDistribution, RegistryManifest, RegistryModelCatalogEntry,
};
use orbitshell::acp::resolve::{AgentKey, AgentSourceKind};
use orbitshell::acp::runtime_prefs::RuntimePreferences;
use serde_json::json;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

static ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(ref value) = self.previous {
            unsafe {
                std::env::set_var(self.key, value);
            }
        } else {
            unsafe {
                std::env::remove_var(self.key);
            }
        }
    }
}

fn with_temp_app_root<F, T>(func: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let _guard = ENV_LOCK.lock().unwrap();
    let temp_dir = TempDir::new()?;
    let _appdata = EnvVarGuard::set("APPDATA", temp_dir.path());
    let _xdg = EnvVarGuard::set("XDG_CONFIG_HOME", temp_dir.path());
    let _home = EnvVarGuard::set("HOME", temp_dir.path());
    func()
}

#[test]
fn runtime_preferences_round_trip_default_model_by_agent_key() -> Result<()> {
    with_temp_app_root(|| {
        let mut preferences = RuntimePreferences::load()?;
        let agent_key = AgentKey {
            source_type: AgentSourceKind::Registry,
            source_id: "managed".into(),
            acp_id: "codex-acp".into(),
        };

        preferences.set_default_model(agent_key.clone(), Some("gpt-5.4".into()))?;
        assert_eq!(
            preferences.default_model_for(&agent_key),
            Some("gpt-5.4".into())
        );

        let mut reloaded = RuntimePreferences::load()?;
        assert_eq!(
            reloaded.default_model_for(&agent_key),
            Some("gpt-5.4".into())
        );

        reloaded.clear_default_model(&agent_key)?;
        assert!(reloaded.default_model_for(&agent_key).is_none());

        let final_load = RuntimePreferences::load()?;
        assert!(final_load.default_model_for(&agent_key).is_none());
        Ok(())
    })
}

#[test]
fn discovered_model_ids_normalize_into_dropdown_options() {
    let caps = json!({
        "modelCatalog": [
            {
                "id": "gpt-5.3",
                "label": "GPT-5.3",
                "description": "desc",
                "isDefault": true
            },
            {
                "modelId": "gpt-5.4",
                "name": "GPT-5.4",
                "description": "desc 2",
                "default": false
            }
        ]
    });

    let models = model_discovery::discover_models(Some(&caps), None).expect("should parse models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "gpt-5.3");
    assert_eq!(models[0].label, "GPT-5.3");
    assert!(models[0].is_default);
    assert_eq!(models[1].id, "gpt-5.4");
    assert_eq!(models[1].label, "GPT-5.4");
    assert!(!models[1].is_default);
}

#[test]
fn malformed_model_metadata_returns_none() {
    let caps = json!({
        "modelCatalog": [
            { "label": "missing id" }
        ]
    });

    assert!(model_discovery::discover_models(Some(&caps), None).is_none());
}

#[test]
fn saved_default_model_cleared_when_missing_in_catalog() -> Result<()> {
    with_temp_app_root(|| {
        let mut preferences = RuntimePreferences::load()?;
        let agent_key = AgentKey {
            source_type: AgentSourceKind::Registry,
            source_id: "managed".into(),
            acp_id: "codex-acp".into(),
        };

        preferences.set_default_model(agent_key.clone(), Some("gpt-5.5".into()))?;

        let caps = json!({
            "modelCatalog": [
                { "id": "gpt-5.3", "label": "GPT-5.3" }
            ]
        });

        let models = model_discovery::discover_models(Some(&caps), None).expect("catalog parsed");
        assert_eq!(models.len(), 1);

        let ids = models
            .iter()
            .map(|model| model.id.clone())
            .collect::<Vec<_>>();
        preferences.ensure_default_model_valid(&agent_key, &ids)?;

        assert!(preferences.default_model_for(&agent_key).is_none());
        Ok(())
    })
}

#[test]
fn session_override_wins_over_persisted_default() {
    let catalog = vec![
        model_discovery::AcpModelOption {
            id: "gpt-5.3".into(),
            label: "GPT-5.3".into(),
            description: None,
            is_default: false,
        },
        model_discovery::AcpModelOption {
            id: "gpt-5.4".into(),
            label: "GPT-5.4".into(),
            description: None,
            is_default: true,
        },
    ];

    let result =
        model_discovery::resolve_selected_model(Some("gpt-5.9"), Some("gpt-5.4"), &catalog);
    assert_eq!(result, Some("gpt-5.9".into()));
}

#[test]
fn persisted_default_wins_over_acp_default() {
    let catalog = vec![model_discovery::AcpModelOption {
        id: "gpt-5.3".into(),
        label: "GPT-5.3".into(),
        description: None,
        is_default: true,
    }];

    let result = model_discovery::resolve_selected_model(None, Some("gpt-5.3"), &catalog);
    assert_eq!(result, Some("gpt-5.3".into()));
}

#[test]
fn acp_default_used_when_no_persisted_default() {
    let catalog = vec![model_discovery::AcpModelOption {
        id: "gpt-5.4".into(),
        label: "GPT-5.4".into(),
        description: None,
        is_default: true,
    }];

    let result = model_discovery::resolve_selected_model(None, None, &catalog);
    assert_eq!(result, Some("gpt-5.4".into()));
}

#[test]
fn no_model_returns_none() {
    let catalog = Vec::new();
    let result = model_discovery::resolve_selected_model(None, None, &catalog);
    assert_eq!(result, None);
}

#[test]
fn registry_manifest_models_convert_to_dropdown_options() {
    let manifest = RegistryManifest {
        id: "codex-acp".into(),
        name: "Codex CLI".into(),
        description: String::new(),
        version: "0.10.0".into(),
        repository: None,
        authors: Vec::new(),
        license: None,
        icon: None,
        distribution: RegistryDistribution::default(),
        model_catalog: vec![
            RegistryModelCatalogEntry {
                id: "gpt-5.3".into(),
                label: "GPT-5.3".into(),
                description: Some("Stable".into()),
                is_default: true,
            },
            RegistryModelCatalogEntry {
                id: "gpt-5.4".into(),
                label: String::new(),
                description: None,
                is_default: false,
            },
        ],
    };

    let models = model_discovery::registry_models(&manifest);

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].label, "GPT-5.3");
    assert!(models[0].is_default);
    assert_eq!(models[1].label, "gpt-5.4");
}
