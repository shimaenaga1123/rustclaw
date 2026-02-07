use super::error::ToolError;
use crate::scheduler::Scheduler;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
pub struct UnscheduleArgs {
    pub task_id: String,
}

#[derive(Clone)]
pub struct Unschedule {
    pub scheduler: Arc<Scheduler>,
    pub is_owner: bool,
}

impl Tool for Unschedule {
    const NAME: &'static str = "unschedule";

    type Error = ToolError;
    type Args = UnscheduleArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Remove a scheduled task by ID (owner only)".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The ID of the scheduled task to remove"
                    }
                },
                "required": ["task_id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if !self.is_owner {
            return Err(ToolError::ScheduleFailed(
                "Permission denied: only the bot owner can remove scheduled tasks".to_string(),
            ));
        }

        let removed = self
            .scheduler
            .remove_task(&args.task_id)
            .await
            .map_err(|e| ToolError::ScheduleFailed(e.to_string()))?;

        if removed {
            Ok(format!("Removed scheduled task: {}", args.task_id))
        } else {
            Ok(format!("Task not found: {}", args.task_id))
        }
    }
}
