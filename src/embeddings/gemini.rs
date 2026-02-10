use crate::embeddings::types;
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use tracing::info;

const GEMINI_DEFAULT_DIM: usize = 768;
const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

pub struct GeminiEmbedding {
    client: reqwest::Client,
    api_key: String,
    model: String,
    dimensions: usize,
}

#[derive(Deserialize)]
struct GeminiEmbedResponse {
    embedding: GeminiEmbeddingValues,
}

#[derive(Deserialize)]
struct GeminiEmbeddingValues {
    values: Vec<f32>,
}

impl GeminiEmbedding {
    pub fn new(api_key: &str, model: Option<&str>, dimensions: Option<usize>) -> Self {
        let model = model.unwrap_or("gemini-embedding-001");
        let dimensions = dimensions.unwrap_or(GEMINI_DEFAULT_DIM);
        info!(
            "Gemini embedding service initialized (model: {}, dim: {})",
            model, dimensions
        );
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dimensions,
        }
    }

    async fn embed(&self, text: &str, task_type: &str) -> Result<Vec<f32>> {
        let url = format!(
            "{}/models/{}:embedContent?key={}",
            GEMINI_BASE_URL, self.model, self.api_key
        );

        let body = serde_json::json!({
            "content": { "parts": [{ "text": text }] },
            "taskType": task_type,
            "outputDimensionality": self.dimensions
        });

        let resp = self.client.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Gemini embedding API error: {} {}", status, body);
        }

        let data: GeminiEmbedResponse = resp.json().await?;
        Ok(data.embedding.values)
    }
}

#[async_trait]
impl types::EmbeddingService for GeminiEmbedding {
    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed_passage(&self, text: &str) -> Result<Vec<f32>> {
        self.embed(text, "RETRIEVAL_DOCUMENT").await
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.embed(text, "RETRIEVAL_QUERY").await
    }
}
