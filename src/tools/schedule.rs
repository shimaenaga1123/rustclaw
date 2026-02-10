use super::error::ToolError;
use crate::scheduler::Scheduler;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
pub struct ScheduleAddArgs {
    pub cron_expr: String,
    pub prompt: String,
    pub description: String,
}

#[derive(Clone)]
pub struct ScheduleAdd {
    pub scheduler: Arc<Scheduler>,
    pub is_owner: bool,
    pub discord_channel_id: Option<u64>,
}

impl Tool for ScheduleAdd {
    const NAME: &'static str = "schedule";

    type Error = ToolError;
    type Args = ScheduleAddArgs;
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

#[derive(Deserialize, Serialize)]
pub struct ScheduleListArgs {}

#[derive(Clone)]
pub struct ScheduleList {
    pub scheduler: Arc<Scheduler>,
}

impl Tool for ScheduleList {
    const NAME: &'static str = "list_schedules";

    type Error = ToolError;
    type Args = ScheduleListArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "List all scheduled tasks".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let tasks = self.scheduler.list_tasks().await;

        if tasks.is_empty() {
            return Ok("No scheduled tasks".to_string());
        }

        let mut output = format!("Scheduled tasks ({}):\n\n", tasks.len());
        for task in tasks {
            output.push_str(&format!(
                "ID: {}\n  Cron: {}\n  Description: {}\n  Prompt: {}\n\n",
                task.id, task.cron_expr, task.description, task.prompt
            ));
        }

        Ok(output)
    }
}

#[derive(Deserialize, Serialize)]
pub struct ScheduleDeleteArgs {
    pub task_id: String,
}

#[derive(Clone)]
pub struct ScheduleDelete {
    pub scheduler: Arc<Scheduler>,
    pub is_owner: bool,
}

impl Tool for ScheduleDelete {
    const NAME: &'static str = "unschedule";

    type Error = ToolError;
    type Args = ScheduleDeleteArgs;
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
