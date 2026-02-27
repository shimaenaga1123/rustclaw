use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub discord: DiscordConfig,
    pub api: ApiConfig,
    pub search: SearchConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub commands: CommandsConfig,
    pub model: ModelConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscordConfig {
    pub token: String,
    pub owner_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiConfig {
    pub provider: String,
    pub key: String,
    pub url: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchConfig {
    pub provider: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct StorageConfig {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("data")
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CommandsConfig {
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ModelConfig {
    #[serde(default = "default_disable_reasoning")]
    pub disable_reasoning: bool,
}

fn default_disable_reasoning() -> bool {
    false
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct EmbeddingConfig {
    #[serde(default = "default_embedding_provider")]
    pub provider: String,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub dimensions: Option<usize>,
}

fn default_embedding_provider() -> String {
    "local".to_string()
}

impl Config {
    pub fn from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path))?;

        let config_file: Config =
            toml::from_str(&content).context("Failed to parse config file")?;

        Ok(config_file)
    }

    pub fn load() -> Result<Self> {
        Self::from_file("config.toml")
    }
}
