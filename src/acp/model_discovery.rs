use crate::acp::client::AcpClient;
use crate::acp::registry::model::RegistryManifest;
use crate::acp::resolve::AgentKey;
use crate::acp::runtime_prefs::RuntimePreferences;
use anyhow::Result;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpModelOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub is_default: bool,
}

pub fn discover_models_from_client(
    client: &AcpClient,
    agent_key: &AgentKey,
    preferences: &mut RuntimePreferences,
) -> Result<Option<Vec<AcpModelOption>>> {
    if let Some(models) = discover_models(
        client.agent_capabilities.as_ref(),
        client.agent_info.as_ref(),
    ) {
        let ids: Vec<String> = models.iter().map(|model| model.id.clone()).collect();
        preferences.ensure_default_model_valid(agent_key, &ids)?;
        return Ok(Some(models));
    }
    Ok(None)
}

pub fn discover_models(
    capabilities: Option<&Value>,
    info: Option<&Value>,
) -> Option<Vec<AcpModelOption>> {
    extract_models(capabilities)
        .or_else(|| extract_models(info))
        .filter(|models| !models.is_empty())
}

pub fn registry_models(manifest: &RegistryManifest) -> Vec<AcpModelOption> {
    manifest
        .model_catalog
        .iter()
        .map(|model| AcpModelOption {
            id: model.id.clone(),
            label: if model.label.is_empty() {
                model.id.clone()
            } else {
                model.label.clone()
            },
            description: model.description.clone(),
            is_default: model.is_default,
        })
        .collect()
}

pub fn resolve_selected_model(
    session_override: Option<&str>,
    persisted_default: Option<&str>,
    catalog: &[AcpModelOption],
) -> Option<String> {
    if let Some(override_model) = session_override {
        return Some(override_model.to_string());
    }

    if let Some(persisted) = persisted_default {
        if catalog.iter().any(|model| model.id == persisted) {
            return Some(persisted.to_string());
        }
    }

    catalog
        .iter()
        .find(|model| model.is_default)
        .map(|model| model.id.clone())
}

fn extract_models(source: Option<&Value>) -> Option<Vec<AcpModelOption>> {
    let value = source?;

    let catalog_keys = ["modelCatalog", "modelSelection"];
    for key in catalog_keys {
        if let Some(models) = value.get(key) {
            if let Some(parsed) = parse_model_array(models) {
                if !parsed.is_empty() {
                    return Some(parsed);
                }
            }
        }
    }

    None
}

fn parse_model_array(array_value: &Value) -> Option<Vec<AcpModelOption>> {
    if let Some(array) = array_value.as_array() {
        let normalized = array
            .iter()
            .filter_map(normalize_model_entry)
            .collect::<Vec<_>>();
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }

    if let Some(object) = array_value.as_object() {
        if let Some(models_value) = object.get("models") {
            return parse_model_array(models_value);
        }
    }

    None
}

fn normalize_model_entry(value: &Value) -> Option<AcpModelOption> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| value.get("modelId").and_then(Value::as_str))
        .map(ToString::to_string)?;

    let label = value
        .get("label")
        .and_then(Value::as_str)
        .or_else(|| value.get("name").and_then(Value::as_str))
        .unwrap_or(&id)
        .to_string();

    let description = value
        .get("description")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let is_default = value
        .get("isDefault")
        .or_else(|| value.get("default"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Some(AcpModelOption {
        id,
        label,
        description,
        is_default,
    })
}
