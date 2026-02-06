use super::error::ToolError;
use crate::memory::MemoryManager;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
pub struct RememberArgs {
    pub content: String,
}

#[derive(Clone)]
pub struct Remember {
    pub memory: Arc<MemoryManager>,
}

impl Tool for Remember {
    const NAME: &'static str = "remember";

    type Error = ToolError;
    type Args = RememberArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Save important information to long-term memory".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "Content to remember"
                    }
                },
                "required": ["content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.memory
            .add_to_long_term(&args.content)
            .await
            .map_err(|e| ToolError::MemoryFailed(e.to_string()))?;
        Ok("Saved".to_string())
    }
}
