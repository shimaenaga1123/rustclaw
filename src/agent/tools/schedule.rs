use super::error::ToolError;
use crate::scheduler::Scheduler;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
pub struct ScheduleArgs {
    pub cron_expr: String,
    pub prompt: String,
    pub description: String,
}

#[derive(Clone)]
pub struct Schedule {
    pub scheduler: Arc<Scheduler>,
    pub is_owner: bool,
    pub discord_channel_id: Option<u64>,
}

impl Tool for Schedule {
    const NAME: &'static str = "schedule";

    type Error = ToolError;
    type Args = ScheduleArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Schedule a recurring task with cron expression".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cron_expr": {
                        "type": "string",
                        "description": "Cron expression (e.g., '0 0 9 * * *' for daily at 9am)"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The prompt/task to execute on schedule"
                    },
                    "description": {
                        "type": "string",
                        "description": "Brief description of the scheduled task"
                    }
                },
                "required": ["cron_expr", "prompt", "description"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let task_id = self
            .scheduler
            .add_task(
                &args.cron_expr,
                &args.prompt,
                &args.description,
                self.is_owner,
                self.discord_channel_id,
            )
            .await
            .map_err(|e| ToolError::ScheduleFailed(e.to_string()))?;

        Ok(format!("Scheduled task created with ID: {}", task_id))
    }
}
