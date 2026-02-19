mod gemini;
mod local;
mod types;

use crate::config::Config;
use anyhow::Result;
pub use gemini::GeminiEmbedding;
pub use local::LocalEmbedding;
use std::sync::Arc;
pub use types::EmbeddingService;

pub async fn create_embedding_service(config: Config) -> Result<Arc<dyn EmbeddingService>> {
    match config.embedding_provider.as_str() {
        "gemini" => {
            let api_key = config
                .embedding_api_key
                .as_deref()
                .unwrap_or(&config.api_key);
            Ok(Arc::new(GeminiEmbedding::new(
                api_key,
                config.embedding_model.as_deref(),
                config.embedding_dimensions,
            )))
        }
        _ => {
            let local = LocalEmbedding::new(&config.data_dir.join("models"))?;
            local.start_unload_timer();
            Ok(Arc::new(local))
        }
    }
}
