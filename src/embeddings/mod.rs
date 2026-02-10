mod gemini;
mod local;
mod types;

pub use types::EmbeddingService;
pub use gemini::GeminiEmbedding;
pub use local::LocalEmbedding;