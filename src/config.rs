use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
struct ConfigFile {
    discord: DiscordConfig,
    api: ApiConfig,
    brave: BraveConfig,
    storage: StorageConfig,
    commands: CommandsConfig,
    model: ModelConfig,
    #[serde(default)]
    embedding: EmbeddingConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordConfig {
    token: String,
    owner_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiConfig {
    provider: String,
    key: String,
    url: String,
    model: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BraveConfig {
    api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct StorageConfig {
    data_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CommandsConfig {
    timeout: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelConfig {
    disable_reasoning: bool,
}

fn default_embedding_provider() -> String {
    "local".to_string()
}

#[derive(Debug, Clone, Deserialize, Default)]
struct EmbeddingConfig {
    #[serde(default = "default_embedding_provider")]
    provider: String,
    api_key: Option<String>,
    model: Option<String>,
    dimensions: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub discord_token: String,
    pub owner_id: u64,
    pub api_provider: String,
    pub api_key: String,
    pub api_url: String,
    pub model: String,
    pub brave_api_key: Option<String>,
    pub data_dir: PathBuf,
    pub command_timeout: u64,
    pub disable_reasoning: bool,
    pub embedding_provider: String,
    pub embedding_api_key: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_dimensions: Option<usize>,
}

impl Config {
    pub fn from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path))?;

        let config_file: ConfigFile =
            toml::from_str(&content).context("Failed to parse config file")?;

        Ok(Self {
            discord_token: config_file.discord.token,
            owner_id: config_file.discord.owner_id,
            api_provider: config_file.api.provider,
            api_key: config_file.api.key,
            api_url: config_file.api.url,
            model: config_file.api.model,
            brave_api_key: config_file.brave.api_key,
            data_dir: config_file.storage.data_dir.into(),
            command_timeout: config_file.commands.timeout,
            disable_reasoning: config_file.model.disable_reasoning,
            embedding_provider: config_file.embedding.provider,
            embedding_api_key: config_file.embedding.api_key,
            embedding_model: config_file.embedding.model,
            embedding_dimensions: config_file.embedding.dimensions,
        })
    }

    pub fn load() -> Result<Self> {
        Self::from_file("config.toml")
    }
}