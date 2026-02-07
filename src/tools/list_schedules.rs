use super::error::ToolError;
use crate::scheduler::Scheduler;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
pub struct ListSchedulesArgs {}

#[derive(Clone)]
pub struct ListSchedules {
    pub scheduler: Arc<Scheduler>,
}

impl Tool for ListSchedules {
    const NAME: &'static str = "list_schedules";

    type Error = ToolError;
    type Args = ListSchedulesArgs;
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
