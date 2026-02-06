use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub discord_token: String,
    pub owner_id: u64,
    pub api_key: String,
    pub api_url: String,
    pub model: String,
    pub brave_api_key: Option<String>,
    pub data_dir: PathBuf,
    pub context_limit: usize,
    pub context_threshold: f32,
    pub command_timeout: u64,
    pub sandbox_image: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            discord_token: std::env::var("DISCORD_TOKEN").context("DISCORD_TOKEN not set")?,
            owner_id: std::env::var("OWNER_ID")
                .context("OWNER_ID not set")?
                .parse()
                .context("OWNER_ID must be a valid Discord user ID")?,
            api_key: std::env::var("API_KEY").context("API_KEY not set")?,
            api_url: std::env::var("API_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com/v1".to_string()),
            model: std::env::var("MODEL")
                .unwrap_or_else(|_| "claude-3-5-sonnet-20241022".to_string()),
            brave_api_key: std::env::var("BRAVE_API_KEY").ok(),
            data_dir: std::env::var("DATA_DIR")
                .unwrap_or_else(|_| "data".to_string())
                .into(),
            context_limit: std::env::var("CONTEXT_LIMIT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(128000),
            context_threshold: 0.8,
            command_timeout: std::env::var("COMMAND_TIMEOUT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
            sandbox_image: std::env::var("SANDBOX_IMAGE").ok(),
        })
    }
}
