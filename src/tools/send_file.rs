use super::error::ToolError;
use super::run_command::RunCommand;
use crate::agent::PendingFile;
use crate::config::Config;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

const MAX_FILE_SIZE: u64 = 8 * 1024 * 1024; // 8MB

#[derive(Deserialize, Serialize)]
pub struct SendFileArgs {
    pub path: String,
    #[serde(default)]
    pub filename: Option<String>,
}

#[derive(Clone)]
pub struct SendFile {
    pub pending_files: Arc<RwLock<Vec<PendingFile>>>,
    pub config: Config,
}

impl Tool for SendFile {
    const NAME: &'static str = "send_file";

    type Error = ToolError;
    type Args = SendFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Send a file from the Docker container's /workspace as a Discord attachment. \
                          First use run_command to create or save a file to /workspace, then use this tool to send it. \
                          Example workflow: run_command to create 'output.txt', then send_file with path 'output.txt'."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path relative to /workspace in the container (e.g. 'output.txt', 'results/data.csv')"
                    },
                    "filename": {
                        "type": "string",
                        "description": "Optional custom filename for the Discord attachment. Defaults to the original filename."
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let rel_path = args.path.trim_start_matches('/');
        if rel_path.is_empty() {
            return Err(ToolError::CommandFailed("Empty file path".to_string()));
        }
        if rel_path.contains("..") {
            return Err(ToolError::CommandFailed(
                "Path traversal not allowed".to_string(),
            ));
        }

        let workspace = RunCommand::workspace_path(&self.config);
        let host_path = workspace.join(rel_path);

        let canonical = host_path
            .canonicalize()
            .map_err(|_| ToolError::CommandFailed(format!("File not found: {}", rel_path)))?;
        let workspace_canonical = workspace
            .canonicalize()
            .map_err(|e| ToolError::CommandFailed(format!("Workspace not accessible: {}", e)))?;
        if !canonical.starts_with(&workspace_canonical) {
            return Err(ToolError::CommandFailed(
                "Path resolves outside workspace".to_string(),
            ));
        }

        let metadata = tokio::fs::metadata(&canonical)
            .await
            .map_err(|_| ToolError::CommandFailed(format!("File not found: {}", rel_path)))?;

        if !metadata.is_file() {
            return Err(ToolError::CommandFailed(format!(
                "'{}' is not a file (directory?)",
                rel_path
            )));
        }

        if metadata.len() > MAX_FILE_SIZE {
            return Err(ToolError::CommandFailed(format!(
                "File too large: {} bytes (max {} MB)",
                metadata.len(),
                MAX_FILE_SIZE / (1024 * 1024)
            )));
        }

        let display_name = args.filename.unwrap_or_else(|| {
            canonical
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string()
        });

        let display_name: String = display_name
            .replace(['/', '\\', '\0'], "_")
            .chars()
            .take(100)
            .collect();

        self.pending_files.write().await.push(PendingFile {
            filename: display_name.clone(),
            path: canonical,
        });

        Ok(format!(
            "File '{}' ({} bytes) queued for sending",
            display_name,
            metadata.len()
        ))
    }
}
