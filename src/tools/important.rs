use super::error::ToolError;
use crate::vectordb::VectorDb;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
pub struct ImportantAddArgs {
    pub content: String,
}

#[derive(Clone)]
pub struct ImportantAdd {
    pub vectordb: Arc<VectorDb>,
    pub is_owner: bool,
}

impl Tool for ImportantAdd {
    const NAME: &'static str = "important_add";

    type Error = ToolError;
    type Args = ImportantAddArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description:
                "Save an important fact to persistent memory (owner only). Use for user preferences, \
                 important dates, key decisions, or anything worth remembering long-term."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The important fact to remember"
                    }
                },
                "required": ["content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if !self.is_owner {
            return Err(ToolError::MemoryFailed(
                "Permission denied: only the bot owner can add important entries".to_string(),
            ));
        }

        let id = self
            .vectordb
            .add_important(&args.content)
            .await
            .map_err(|e| ToolError::MemoryFailed(e.to_string()))?;

        Ok(format!("Saved (ID: {}): {}", id, args.content))
    }
}

#[derive(Deserialize, Serialize)]
pub struct ImportantListArgs {}

#[derive(Clone)]
pub struct ImportantList {
    pub vectordb: Arc<VectorDb>,
}

impl Tool for ImportantList {
    const NAME: &'static str = "important_list";

    type Error = ToolError;
    type Args = ImportantListArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "List all important facts stored in memory".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let entries = self
            .vectordb
            .list_important()
            .await
            .map_err(|e| ToolError::MemoryFailed(e.to_string()))?;

        if entries.is_empty() {
            return Ok("No important entries stored.".to_string());
        }

        let mut output = format!("Important entries ({}):\n\n", entries.len());
        for entry in &entries {
            let ts = chrono::DateTime::from_timestamp_micros(entry.timestamp_micros)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "unknown".to_string());
            output.push_str(&format!(
                "ID: {} | {}\n  {}\n\n",
                entry.id, ts, entry.content
            ));
        }

        Ok(output)
    }
}

#[derive(Deserialize, Serialize)]
pub struct ImportantDeleteArgs {
    pub id: String,
}

#[derive(Clone)]
pub struct ImportantDelete {
    pub vectordb: Arc<VectorDb>,
    pub is_owner: bool,
}

impl Tool for ImportantDelete {
    const NAME: &'static str = "important_delete";

    type Error = ToolError;
    type Args = ImportantDeleteArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Delete an important entry by ID (owner only)".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The ID of the important entry to delete"
                    }
                },
                "required": ["id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if !self.is_owner {
            return Err(ToolError::MemoryFailed(
                "Permission denied: only the bot owner can delete important entries".to_string(),
            ));
        }

        self.vectordb
            .delete_important(&args.id)
            .await
            .map_err(|e| ToolError::MemoryFailed(e.to_string()))?;

        Ok(format!("Deleted important entry: {}", args.id))
    }
}
