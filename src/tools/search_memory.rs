use super::error::ToolError;
use crate::vector_db::VectorDb;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
pub struct SearchMemoryArgs {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

fn default_top_k() -> usize {
    5
}

#[derive(Clone)]
pub struct SearchMemory {
    pub vectordb: Arc<VectorDb>,
}

impl Tool for SearchMemory {
    const NAME: &'static str = "search_memory";

    type Error = ToolError;
    type Args = SearchMemoryArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description:
            "Search past conversations semantically. Returns the most relevant past conversation turns matching the query."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language search query"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "Number of results to return (default: 5, max: 20)",
                        "default": 5
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let top_k = args.top_k.clamp(1, 20);

        let results = self
            .vectordb
            .search_turns(&args.query, top_k, &[])
            .await
            .map_err(|e| ToolError::MemoryFailed(e.to_string()))?;

        if results.is_empty() {
            return Ok("No relevant conversations found.".to_string());
        }

        let mut output = format!("Found {} relevant conversations:\n\n", results.len());
        for turn in &results {
            let ts = chrono::DateTime::from_timestamp_micros(turn.timestamp_micros)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "unknown".to_string());
            output.push_str(&format!(
                "[{}] {}: {}\nAssistant: {}\n\n",
                ts, turn.author, turn.user_input, turn.assistant_response
            ));
        }

        Ok(output)
    }
}
