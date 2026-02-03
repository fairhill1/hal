use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub default_provider: String,
    pub mode: Mode,
    pub providers: HashMap<String, Provider>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Coding,
    Coach,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        let config_path = Self::config_path();

        if config_path.exists() {
            match fs::read_to_string(&config_path) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(config) => return config,
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to parse config at {}: {}",
                            config_path.display(),
                            e
                        );
                        eprintln!("Using default configuration.");
                    }
                },
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to read config at {}: {}",
                        config_path.display(),
                        e
                    );
                    eprintln!("Using default configuration.");
                }
            }
        }

        let default = Self::default();
        let _ = default.save();
        default
    }

    pub fn save(&self) -> Result<(), String> {
        let config_path = Self::config_path();

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let content = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&config_path, content).map_err(|e| e.to_string())?;

        Ok(())
    }

    pub fn get_provider(&self) -> Option<&Provider> {
        self.providers.get(&self.default_provider)
    }

    fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("hal")
            .join("config.json")
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut providers = HashMap::new();

        providers.insert(
            "gemini".to_string(),
            Provider {
                base_url: "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
                model: "gemini-3-flash-preview".to_string(),
                api_key_env: "HAL_API_KEY_GEMINI".to_string(),
                api_key: None,
            },
        );

        providers.insert(
            "openai".to_string(),
            Provider {
                base_url: "https://api.openai.com/v1".to_string(),
                model: "gpt-4o".to_string(),
                api_key_env: "HAL_API_KEY_OPENAI".to_string(),
                api_key: None,
            },
        );

        providers.insert(
            "anthropic".to_string(),
            Provider {
                base_url: "https://api.anthropic.com/v1".to_string(),
                model: "claude-sonnet-4-20250514".to_string(),
                api_key_env: "HAL_API_KEY_ANTHROPIC".to_string(),
                api_key: None,
            },
        );

        providers.insert(
            "openrouter".to_string(),
            Provider {
                base_url: "https://openrouter.ai/api/v1".to_string(),
                model: "anthropic/claude-sonnet-4".to_string(),
                api_key_env: "HAL_API_KEY_OPENROUTER".to_string(),
                api_key: None,
            },
        );

        Config {
            default_provider: "gemini".to_string(),
            mode: Mode::Coding,
            providers,
        }
    }
}
