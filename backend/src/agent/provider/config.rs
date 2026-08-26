use super::{
    openai::{ChatCompatibility, OpenAiChatProvider, OpenAiResponsesProvider},
    FakeProvider, LlmProvider, ProviderError,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

pub const PROVIDER_LIST_SCHEMA_V1: &str = "agent-provider-list/v1";
const MAX_CONFIG_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAdapterKind {
    Fake,
    OpenaiResponses,
    OpenaiCompatibleChat,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    pub id: String,
    pub label: String,
    pub adapter: ProviderAdapterKind,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub chat_compatibility: Option<ChatCompatibility>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfigFile {
    #[serde(default)]
    profiles: Vec<ProviderProfile>,
}

#[derive(Debug, Clone)]
pub struct ProviderCatalog {
    profiles: Vec<ProviderProfile>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SafeProviderProfile {
    pub id: String,
    pub label: String,
    pub model: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderListResponse {
    pub schema_version: &'static str,
    pub profiles: Vec<SafeProviderProfile>,
}

impl ProviderCatalog {
    pub fn offline_default() -> Self {
        Self {
            profiles: vec![ProviderProfile {
                id: "offline".to_string(),
                label: "离线测试".to_string(),
                adapter: ProviderAdapterKind::Fake,
                model: "deterministic-fixture-v1".to_string(),
                base_url: None,
                api_key_env: None,
                enabled: true,
                chat_compatibility: None,
            }],
        }
    }

    pub fn load_from_env() -> Result<Self, ProviderError> {
        let Some(path) = std::env::var_os("JX3_AGENT_CONFIG") else {
            return Ok(Self::offline_default());
        };
        Self::load_from_path(Path::new(&path))
    }

    pub fn load_from_path(path: &Path) -> Result<Self, ProviderError> {
        let metadata = std::fs::metadata(path).map_err(|_| {
            ProviderError::configuration(
                "provider_config_unreadable",
                "provider configuration cannot be read",
            )
        })?;
        if metadata.len() > MAX_CONFIG_BYTES {
            return Err(ProviderError::configuration(
                "provider_config_too_large",
                "provider configuration exceeds 262144 bytes",
            ));
        }
        let source = std::fs::read_to_string(path).map_err(|_| {
            ProviderError::configuration(
                "provider_config_unreadable",
                "provider configuration cannot be read",
            )
        })?;
        Self::from_toml(&source)
    }

    pub fn from_toml(source: &str) -> Result<Self, ProviderError> {
        let config: ProviderConfigFile = toml::from_str(source).map_err(|_| {
            ProviderError::configuration(
                "provider_config_invalid",
                "provider configuration is not valid TOML",
            )
        })?;
        validate_profiles(&config.profiles)?;
        Ok(Self {
            profiles: config.profiles,
        })
    }

    pub fn safe_list(&self) -> ProviderListResponse {
        self.safe_list_with(|name| std::env::var(name).ok())
    }

    fn safe_list_with<F>(&self, resolve: F) -> ProviderListResponse
    where
        F: Fn(&str) -> Option<String>,
    {
        ProviderListResponse {
            schema_version: PROVIDER_LIST_SCHEMA_V1,
            profiles: self
                .profiles
                .iter()
                .map(|profile| SafeProviderProfile {
                    id: profile.id.clone(),
                    label: profile.label.clone(),
                    model: profile.model.clone(),
                    available: profile.enabled
                        && match profile.adapter {
                            ProviderAdapterKind::Fake => true,
                            _ => profile
                                .api_key_env
                                .as_deref()
                                .and_then(&resolve)
                                .is_some_and(|value| !value.trim().is_empty()),
                        },
                })
                .collect(),
        }
    }

    pub fn create_provider(&self, profile_id: &str) -> Result<Box<dyn LlmProvider>, ProviderError> {
        let profile = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| {
                ProviderError::configuration("provider_not_found", "provider profile was not found")
            })?;
        if !profile.enabled {
            return Err(ProviderError::configuration(
                "provider_disabled",
                "provider profile is disabled",
            ));
        }
        match profile.adapter {
            ProviderAdapterKind::Fake => Ok(Box::new(FakeProvider::new(
                profile.id.clone(),
                profile.model.clone(),
            ))),
            ProviderAdapterKind::OpenaiResponses => {
                let (base_url, api_key) = network_credentials(profile)?;
                Ok(Box::new(OpenAiResponsesProvider::new(
                    profile.id.clone(),
                    profile.model.clone(),
                    base_url,
                    api_key,
                )?))
            }
            ProviderAdapterKind::OpenaiCompatibleChat => {
                let (base_url, api_key) = network_credentials(profile)?;
                Ok(Box::new(OpenAiChatProvider::new_with_compatibility(
                    profile.id.clone(),
                    profile.model.clone(),
                    base_url,
                    api_key,
                    profile.chat_compatibility.unwrap_or_default(),
                )?))
            }
        }
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }
}

fn enabled_by_default() -> bool {
    true
}

fn network_credentials(profile: &ProviderProfile) -> Result<(String, String), ProviderError> {
    let base_url = profile.base_url.clone().ok_or_else(|| {
        ProviderError::configuration("provider_base_url_missing", "provider base URL is missing")
    })?;
    let key_env = profile.api_key_env.as_deref().ok_or_else(|| {
        ProviderError::configuration(
            "provider_key_env_missing",
            "provider key environment name is missing",
        )
    })?;
    let key = std::env::var(key_env).map_err(|_| {
        ProviderError::configuration(
            "provider_key_unavailable",
            "provider credential is unavailable",
        )
    })?;
    if key.trim().is_empty() {
        return Err(ProviderError::configuration(
            "provider_key_unavailable",
            "provider credential is unavailable",
        ));
    }
    Ok((base_url, key))
}

fn validate_profiles(profiles: &[ProviderProfile]) -> Result<(), ProviderError> {
    if profiles.is_empty() || profiles.len() > 16 {
        return Err(ProviderError::configuration(
            "provider_profile_count_invalid",
            "provider configuration requires 1..16 profiles",
        ));
    }
    let mut ids = HashSet::new();
    for profile in profiles {
        if !super::protocol::valid_identifier(&profile.id) || !ids.insert(profile.id.as_str()) {
            return Err(ProviderError::configuration(
                "provider_profile_id_invalid",
                "provider profile ids must be unique safe identifiers",
            ));
        }
        if profile.label.trim().is_empty()
            || profile.label.len() > 80
            || profile.model.trim().is_empty()
            || profile.model.len() > 128
        {
            return Err(ProviderError::configuration(
                "provider_profile_text_invalid",
                "provider labels and models must be non-empty and bounded",
            ));
        }
        match profile.adapter {
            ProviderAdapterKind::Fake => {
                if profile.base_url.is_some()
                    || profile.api_key_env.is_some()
                    || profile.chat_compatibility.is_some()
                {
                    return Err(ProviderError::configuration(
                        "fake_provider_has_credentials",
                        "fake provider must not define a URL or credential",
                    ));
                }
            }
            ProviderAdapterKind::OpenaiResponses => {
                validate_base_url(profile.base_url.as_deref())?;
                if !profile.api_key_env.as_deref().is_some_and(valid_env_name) {
                    return Err(ProviderError::configuration(
                        "provider_key_env_invalid",
                        "provider key environment name is invalid",
                    ));
                }
                if profile.chat_compatibility.is_some() {
                    return Err(ProviderError::configuration(
                        "provider_chat_compatibility_invalid",
                        "chat compatibility is only valid for compatible Chat providers",
                    ));
                }
            }
            ProviderAdapterKind::OpenaiCompatibleChat => {
                validate_base_url(profile.base_url.as_deref())?;
                if !profile.api_key_env.as_deref().is_some_and(valid_env_name) {
                    return Err(ProviderError::configuration(
                        "provider_key_env_invalid",
                        "provider key environment name is invalid",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_base_url(value: Option<&str>) -> Result<(), ProviderError> {
    let value = value.ok_or_else(|| {
        ProviderError::configuration("provider_base_url_missing", "provider base URL is missing")
    })?;
    let url = reqwest::Url::parse(value).map_err(|_| {
        ProviderError::configuration("provider_base_url_invalid", "provider base URL is invalid")
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderError::configuration(
            "provider_base_url_invalid",
            "provider base URL is invalid",
        ));
    }
    if url.scheme() == "http" && !url.host_str().is_some_and(is_loopback_host) {
        return Err(ProviderError::configuration(
            "provider_insecure_remote_url",
            "remote provider base URL must use HTTPS",
        ));
    }
    Ok(())
}

pub(super) fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

fn valid_env_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && value.len() <= 128
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
[[profiles]]
id = "offline"
label = "Offline"
adapter = "fake"
model = "fixture-v1"

[[profiles]]
id = "openai"
label = "OpenAI"
adapter = "openai_responses"
model = "example-model"
base_url = "https://api.openai.com/v1"
api_key_env = "EXAMPLE_OPENAI_KEY"
"#;

    #[test]
    fn safe_list_never_exposes_adapter_url_or_key_name() {
        let catalog = ProviderCatalog::from_toml(CONFIG).unwrap();
        let list = catalog.safe_list_with(|name| {
            (name == "EXAMPLE_OPENAI_KEY").then(|| "secret-value".to_string())
        });
        let json = serde_json::to_string(&list).unwrap();
        assert_eq!(list.profiles.len(), 2);
        assert!(list.profiles[0].available);
        assert!(list.profiles[1].available);
        assert!(!json.contains("secret-value"));
        assert!(!json.contains("EXAMPLE_OPENAI_KEY"));
        assert!(!json.contains("api.openai.com"));
        assert!(!json.contains("openai_responses"));
    }

    #[test]
    fn unavailable_key_and_invalid_profiles_are_handled_without_secret_values() {
        let catalog = ProviderCatalog::from_toml(CONFIG).unwrap();
        let list = catalog.safe_list_with(|_| None);
        assert!(list.profiles[0].available);
        assert!(!list.profiles[1].available);

        let duplicate = CONFIG.replace("id = \"openai\"", "id = \"offline\"");
        assert_eq!(
            ProviderCatalog::from_toml(&duplicate).unwrap_err().code,
            "provider_profile_id_invalid"
        );
    }

    #[test]
    fn default_catalog_is_offline_and_requires_no_environment() {
        let catalog = ProviderCatalog::offline_default();
        let list = catalog.safe_list_with(|_| None);
        assert_eq!(catalog.len(), 1);
        assert_eq!(list.profiles[0].id, "offline");
        assert!(list.profiles[0].available);
    }

    #[test]
    fn plaintext_http_is_limited_to_loopback_mock_or_local_providers() {
        let insecure = CONFIG.replace("https://api.openai.com/v1", "http://provider.example/v1");
        assert_eq!(
            ProviderCatalog::from_toml(&insecure).unwrap_err().code,
            "provider_insecure_remote_url"
        );

        let local = CONFIG.replace("https://api.openai.com/v1", "http://127.0.0.1:8080/v1");
        ProviderCatalog::from_toml(&local).unwrap();
    }

    #[test]
    fn checked_in_example_config_remains_parseable_and_safe_by_default() {
        let source = include_str!("../../../../config/agent.providers.example.toml");
        let catalog = ProviderCatalog::from_toml(source).unwrap();
        let list = catalog.safe_list_with(|_| None);

        assert_eq!(list.profiles.len(), 4);
        assert!(list.profiles[0].available);
        assert!(list.profiles[1..].iter().all(|profile| !profile.available));
    }
}
