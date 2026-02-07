mod agent;
mod config;
mod discord;
mod memory;
mod scheduler;

use anyhow::Result;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = config::Config::load()?;

    let memory_manager = memory::MemoryManager::new(&config.data_dir).await?;
    let agent = agent::RigAgent::new(config.clone(), memory_manager.clone()).await?;

    let scheduler = scheduler::Scheduler::new(&config.data_dir, agent.clone()).await?;
    agent.set_scheduler(scheduler.clone()).await;
    scheduler.start().await?;

    let discord_bot = discord::Bot::new(config, agent, memory_manager, scheduler).await?;
    discord_bot.start().await?;

    Ok(())
}
