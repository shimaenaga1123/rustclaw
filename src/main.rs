mod agent;
mod config;
mod discord;
mod memory;
mod scheduler;
mod tools;
mod utils;

use anyhow::Result;
use tracing::{info, warn};
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
    let agent = agent::create_agent(config.clone(), memory_manager.clone()).await?;

    let scheduler = scheduler::Scheduler::new(&config.data_dir, agent.clone()).await?;
    agent.set_scheduler(scheduler.clone()).await;
    scheduler.start().await?;

    let discord_bot =
        discord::Bot::new(config, agent, memory_manager.clone(), scheduler.clone()).await?;

    let bot_handle = tokio::spawn(async move {
        if let Err(e) = discord_bot.start().await {
            tracing::error!("Discord bot error: {}", e);
        }
    });

    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received, saving state...");

    if let Err(e) = memory_manager.flush().await {
        warn!("Failed to flush memory on shutdown: {}", e);
    }

    if let Err(e) = scheduler.shutdown().await {
        warn!("Failed to shutdown scheduler: {}", e);
    }

    bot_handle.abort();
    info!("Shutdown complete");

    Ok(())
}
