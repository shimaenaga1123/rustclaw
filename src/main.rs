mod agent;
mod config;
mod discord;
mod embeddings;
mod memory;
mod scheduler;
mod tools;
mod utils;
mod vectordb;

use anyhow::Result;
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use embeddings::EmbeddingService;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = config::Config::load()?;

    let embedding_service: Arc<dyn EmbeddingService> = match config.embedding_provider.as_str() {
        "gemini" => {
            let api_key = config
                .embedding_api_key
                .as_deref()
                .unwrap_or(&config.api_key);
            Arc::new(embeddings::GeminiEmbedding::new(
                api_key,
                config.embedding_model.as_deref(),
                config.embedding_dimensions,
            ))
        }
        _ => {
            let local = embeddings::LocalEmbedding::new(&config.data_dir.join("models"))?;
            local.start_unload_timer();
            Arc::new(local)
        }
    };

    let vectordb = vectordb::VectorDb::new(&config.data_dir, embedding_service).await?;
    let memory_manager = memory::MemoryManager::new(vectordb.clone()).await?;

    let agent = agent::create_agent(config.clone(), memory_manager.clone()).await?;

    let scheduler = scheduler::Scheduler::new(&config.data_dir, agent.clone()).await?;
    agent.set_scheduler(scheduler.clone()).await;
    scheduler.start().await?;

    let scheduler_ref = scheduler.clone();
    let bot_handle = tokio::spawn(async move {
        if let Err(e) = discord::setup(config, agent, scheduler_ref).await {
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
