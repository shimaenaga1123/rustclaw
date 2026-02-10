use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait EmbeddingService: Send + Sync {
    fn dimensions(&self) -> usize;
    async fn embed_passage(&self, text: &str) -> Result<Vec<f32>>;
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}
