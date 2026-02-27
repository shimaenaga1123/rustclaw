mod agent;
mod config;
mod discord;
mod embeddings;
mod entity;
mod memory;
mod scheduler;
mod tools;
mod vector_db;

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

    let embedding_service = embeddings::create_embedding_service(config.clone()).await?;

    let vector_db = vector_db::VectorDb::new(&config.storage.data_dir, embedding_service).await?;
    let memory_manager = memory::MemoryManager::new(vector_db.clone()).await?;

    let agent = agent::create_agent(config.clone(), memory_manager.clone()).await?;

    let scheduler = scheduler::Scheduler::new(&config.storage.data_dir, agent.clone()).await?;
    agent.set_scheduler(scheduler.clone()).await;
    scheduler.start().await?;

    let discord_bot = discord::Bot::new(config, agent, scheduler.clone()).await?;
    let bot_handle = tokio::spawn(async move {
        if let Err(e) = discord_bot.start().await {
            tracing::error!("Discord bot error: {}", e);
        }
    });

    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received, saving state...");

    if let Err(e) = scheduler.shutdown().await {
        warn!("Failed to shutdown scheduler: {}", e);
    }

    bot_handle.abort();
    info!("Shutdown complete");

    Ok(())
}
