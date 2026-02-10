use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::info;
use anyhow::{Context, Result};
use async_trait::async_trait;
use crate::embeddings::types::EmbeddingService;

const LOCAL_DIM: usize = 384;
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

pub struct LocalEmbedding {
    model: Arc<Mutex<Option<TextEmbedding>>>,
    cache_dir: PathBuf,
    last_used: Arc<Mutex<Instant>>,
}

impl LocalEmbedding {
    pub fn new(cache_dir: &Path) -> Result<Self> {
        info!(
            "Local embedding service initialized (lazy loading, {}s idle timeout)",
            IDLE_TIMEOUT.as_secs()
        );
        Ok(Self {
            model: Arc::new(Mutex::new(None)),
            cache_dir: cache_dir.to_path_buf(),
            last_used: Arc::new(Mutex::new(Instant::now())),
        })
    }

    pub fn start_unload_timer(&self) {
        let model = self.model.clone();
        let last_used = self.last_used.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let elapsed = last_used.lock().unwrap().elapsed();
                if elapsed >= IDLE_TIMEOUT {
                    let mut guard = model.lock().unwrap();
                    if guard.is_some() {
                        *guard = None;
                        info!("Embedding model unloaded (idle for {}s)", elapsed.as_secs());
                    }
                }
            }
        });
    }
}

#[async_trait]
impl EmbeddingService for LocalEmbedding {
    fn dimensions(&self) -> usize {
        LOCAL_DIM
    }

    async fn embed_passage(&self, text: &str) -> Result<Vec<f32>> {
        let text = format!("passage: {}", text);
        let model = self.model.clone();
        let cache_dir = self.cache_dir.clone();
        let last_used = self.last_used.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = model.lock().unwrap();
            ensure_loaded(&mut guard, &cache_dir)?;
            *last_used.lock().unwrap() = Instant::now();
            let result = guard.as_mut().unwrap().embed(vec![text], None)?;
            Ok(result.into_iter().next().unwrap())
        })
        .await?
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let text = format!("query: {}", text);
        let model = self.model.clone();
        let cache_dir = self.cache_dir.clone();
        let last_used = self.last_used.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = model.lock().unwrap();
            ensure_loaded(&mut guard, &cache_dir)?;
            *last_used.lock().unwrap() = Instant::now();
            let result = guard.as_mut().unwrap().embed(vec![text], None)?;
            Ok(result.into_iter().next().unwrap())
        })
        .await?
    }
}

fn ensure_loaded(model: &mut Option<TextEmbedding>, cache_dir: &Path) -> Result<()> {
    if model.is_none() {
        info!("Loading embedding model (multilingual-e5-small)...");
        *model = Some(
            TextEmbedding::try_new(
                InitOptions::new(EmbeddingModel::MultilingualE5Small)
                    .with_cache_dir(cache_dir.to_path_buf())
                    .with_show_download_progress(true),
            )
            .context("Failed to initialize embedding model")?,
        );
        info!("Embedding model ready");
    }
    Ok(())
}
