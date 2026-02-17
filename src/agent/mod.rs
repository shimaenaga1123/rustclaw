use crate::config::Config;
use crate::memory::MemoryManager;
use anyhow::Result;
pub use attachment::{AttachmentInfo, PendingFile};
use rig::providers::{anthropic, gemini, openai};
use rig_agent::RigAgent;
pub use rig_agent::{Agent, StreamEvent};
use std::sync::Arc;
pub use user_info::UserInfo;

mod attachment;
mod preamble;
mod rig_agent;
mod user_info;

pub async fn create_agent(config: Config, memory: Arc<MemoryManager>) -> Result<Arc<dyn Agent>> {
    match config.api_provider.as_str() {
        "openai" => {
            let client: openai::CompletionsClient = openai::CompletionsClient::builder()
                .api_key(&config.api_key)
                .base_url(&config.api_url)
                .build()?;
            let agent = RigAgent::new(config, memory, client).await?;
            Ok(agent as Arc<dyn Agent>)
        }
        "gemini" => {
            let client = gemini::Client::new(&config.api_key)?;
            let agent = RigAgent::new(config, memory, client).await?;
            Ok(agent as Arc<dyn Agent>)
        }
        _ => {
            let client: anthropic::Client = anthropic::Client::builder()
                .api_key(&config.api_key)
                .base_url(&config.api_url)
                .build()?;
            let agent = RigAgent::new(config, memory, client).await?;
            Ok(agent as Arc<dyn Agent>)
        }
    }
}
