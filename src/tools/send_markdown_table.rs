use super::error::ToolError;
use super::run_command::RunCommand;
use crate::agent::PendingFile;
use crate::config::Config;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
pub struct SendMarkdownTableArgs {
    pub markdown: String,
    #[serde(default)]
    pub filename: Option<String>,
}

#[derive(Clone)]
pub struct SendMarkdownTable {
    pub pending_files: Arc<RwLock<Vec<PendingFile>>>,
    pub config: Config,
}

impl Tool for SendMarkdownTable {
    const NAME: &'static str = "send_markdown_table";

    type Error = ToolError;
    type Args = SendMarkdownTableArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Render a markdown table as an image and send it as a Discord attachment. \
                          Use this when you need to display tables, since Discord doesn't support markdown tables. \
                          The markdown will be converted to a dark-themed image using mdimg."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "markdown": {
                        "type": "string",
                        "description": "Markdown content containing the table to render"
                    },
                    "filename": {
                        "type": "string",
                        "description": "Optional filename for the output image (without extension). Defaults to 'table'."
                    }
                },
                "required": ["markdown"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let workspace = RunCommand::workspace_path(&self.config);
        let tmp_dir = workspace.join("tmp");

        tokio::fs::create_dir_all(&tmp_dir)
            .await
            .map_err(|e| ToolError::CommandFailed(format!("Failed to create tmp dir: {}", e)))?;

        let id = Uuid::new_v4().to_string()[..8].to_string();
        let md_filename = format!("table_{}.md", id);
        let png_filename = format!("{}.png", args.filename.as_deref().unwrap_or("table"));

        let md_path = tmp_dir.join(&md_filename);
        let png_path = tmp_dir.join(&png_filename);

        tokio::fs::write(&md_path, &args.markdown)
            .await
            .map_err(|e| ToolError::CommandFailed(format!("Failed to write markdown: {}", e)))?;

        let run_cmd = RunCommand {
            config: self.config.clone(),
            is_owner: true,
        };

        let command = format!(
            "which mdimg || bun install -g mdimg && mdimg -i /workspace/tmp/{} -o /workspace/tmp/{} --theme dark",
            md_filename, png_filename
        );

        run_cmd
            .call(super::run_command::RunCommandArgs { command })
            .await?;

        if !png_path.exists() {
            return Err(ToolError::CommandFailed(
                "mdimg failed to generate image".to_string(),
            ));
        }

        let metadata = tokio::fs::metadata(&png_path)
            .await
            .map_err(|e| ToolError::CommandFailed(format!("Failed to read output: {}", e)))?;

        self.pending_files.write().await.push(PendingFile {
            filename: png_filename.clone(),
            path: png_path,
        });

        let _ = tokio::fs::remove_file(&md_path).await;

        Ok(format!(
            "Table rendered as '{}' ({} bytes) and queued for sending",
            png_filename,
            metadata.len()
        ))
    }
}
