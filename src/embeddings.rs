use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::info;

pub const EMBEDDING_DIM: i32 = 384;

pub struct EmbeddingService {
    model: Arc<Mutex<TextEmbedding>>,
}

impl EmbeddingService {
    pub fn new(cache_dir: &Path) -> Result<Self> {
        info!("Initializing embedding model (multilingual-e5-small)...");

        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::MultilingualE5Small)
                .with_cache_dir(cache_dir.to_path_buf())
                .with_show_download_progress(true),
        )
        .context("Failed to initialize embedding model")?;

        info!("Embedding model ready");

        Ok(Self {
            model: Arc::new(Mutex::new(model)),
        })
    }

    pub async fn embed_passage(&self, text: &str) -> Result<Vec<f32>> {
        let text = format!("passage: {}", text);
        let model = self.model.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut model = model.lock().unwrap();
            model.embed(vec![text], None)
        })
        .await
        .context("Embedding task panicked")??;
        Ok(result.into_iter().next().unwrap())
    }

    pub async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let text = format!("query: {}", text);
        let model = self.model.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut model = model.lock().unwrap();
            model.embed(vec![text], None)
        })
        .await
        .context("Embedding task panicked")??;
        Ok(result.into_iter().next().unwrap())
    }
}
