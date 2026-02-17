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
pub struct TypstRenderArgs {
    pub content: String,
    #[serde(default)]
    pub filename: Option<String>,
}

#[derive(Clone)]
pub struct TypstRender {
    pub pending_files: Arc<RwLock<Vec<PendingFile>>>,
    pub config: Config,
}

impl TypstRender {
    async fn exec_in_container(config: &Config, command: &str) -> Result<String, ToolError> {
        let runner = RunCommand {
            config: config.clone(),
        };
        runner
            .call(super::run_command::RunCommandArgs {
                command: command.to_string(),
            })
            .await
    }
}

impl Tool for TypstRender {
    const NAME: &'static str = "typst_render";

    type Error = ToolError;
    type Args = TypstRenderArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Render Typst markup to a PNG image and send as a Discord attachment. \
                          Use for tables, math equations, formatted documents, and anything \
                          Discord markdown cannot render."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "Typst markup content to render"
                    },
                    "filename": {
                        "type": "string",
                        "description": "Optional filename (without extension). Defaults to 'render'."
                    }
                },
                "required": ["content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let id = &Uuid::new_v4().to_string()[..8];
        let src_path = format!("/workspace/.typst_{}.typ", id);
        let out_path = format!("/workspace/.typst_{}.png", id);

        let wrapped = format!(
            "#set page(width: auto, height: auto, margin: 16pt, fill: rgb(\"#313338\"))\n\
             #set text(fill: rgb(\"#e0e0e0\"), size: 11pt)\n\
             #set table(stroke: rgb(\"#555\"))\n\n\
             {}",
            args.content
        );

        let escaped = wrapped.replace('\\', "\\\\").replace('\'', "'\\''");
        let write_cmd = format!("printf '%s' '{}' > {}", escaped, src_path);
        Self::exec_in_container(&self.config, &write_cmd).await?;

        Self::exec_in_container(
            &self.config,
            "command -v typst >/dev/null 2>&1 || \
             (apt-get update -qq && apt-get install -y -qq wget >/dev/null 2>&1 && \
              wget -qO /tmp/typst.tar.xz https://github.com/typst/typst/releases/latest/download/typst-x86_64-unknown-linux-musl.tar.xz && \
              tar -xf /tmp/typst.tar.xz -C /tmp && \
              cp /tmp/typst-*/typst /usr/local/bin/ && \
              rm -rf /tmp/typst*)",
        ).await?;

        let compile_cmd = format!("typst compile --ppi 288 {} {}", src_path, out_path);
        Self::exec_in_container(&self.config, &compile_cmd)
            .await
            .map_err(|e| ToolError::CommandFailed(format!("Typst compilation failed: {}", e)))?;

        let cleanup = format!("rm -f {}", src_path);
        let _ = Self::exec_in_container(&self.config, &cleanup).await;

        let filename = format!("{}.png", args.filename.as_deref().unwrap_or("render"));
        let workspace = RunCommand::workspace_path(&self.config);
        let host_file = workspace.join(format!(".typst_{}.png", id));

        let meta = tokio::fs::metadata(&host_file)
            .await
            .map_err(|_| ToolError::CommandFailed("Rendered PNG not found".to_string()))?;

        self.pending_files.write().await.push(PendingFile {
            filename: filename.clone(),
            path: host_file,
        });

        Ok(format!("Rendered '{}' ({} bytes)", filename, meta.len()))
    }
}
