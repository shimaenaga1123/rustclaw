use super::error::ToolError;
use crate::config::Config;
use bollard::{
    Docker,
    container::{
        Config as ContainerConfig, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
        StartContainerOptions, WaitContainerOptions,
    },
};
use futures_util::StreamExt;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use tracing::debug;

fn get_default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

fn get_shell_name(shell_path: &str) -> &str {
    shell_path.rsplit('/').next().unwrap_or("sh")
}

#[derive(Deserialize, Serialize)]
pub struct RunCommandArgs {
    pub command: String,
}

#[derive(Clone)]
pub struct RunCommand {
    pub config: Config,
    pub is_owner: bool,
}

impl RunCommand {
    async fn run_local(&self, command: &str) -> Result<String, ToolError> {
        let shell = get_default_shell();
        debug!("Executing locally with {}: {}", shell, command);

        let output = tokio::time::timeout(
            Duration::from_secs(self.config.command_timeout),
            TokioCommand::new(&shell)
                .arg("-c")
                .arg(command)
                .current_dir(&self.config.data_dir)
                .output(),
        )
        .await
        .map_err(|_| ToolError::Timeout)??;

        Self::process_output(output)
    }

    async fn run_sandbox(&self, command: &str) -> Result<String, ToolError> {
        debug!("Executing in sandbox: {}", command);

        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| ToolError::CommandFailed(format!("Docker connection failed: {}", e)))?;

        let container_name = format!("sandbox-{}", uuid::Uuid::new_v4());
        let image = self
            .config
            .sandbox_image
            .as_deref()
            .unwrap_or("jdxcode/mise:latest");

        let config = ContainerConfig {
            image: Some(image),
            cmd: Some(vec!["sh", "-c", command]),
            working_dir: Some("/workspace"),
            network_disabled: Some(false),
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: container_name.as_str(),
            platform: None,
        };

        docker
            .create_container(Some(options), config)
            .await
            .map_err(|e| ToolError::CommandFailed(format!("Container creation failed: {}", e)))?;

        docker
            .start_container(&container_name, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| ToolError::CommandFailed(format!("Container start failed: {}", e)))?;

        let wait_result = tokio::time::timeout(
            Duration::from_secs(self.config.command_timeout),
            docker
                .wait_container(&container_name, None::<WaitContainerOptions<String>>)
                .next(),
        )
        .await;

        let logs_options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            ..Default::default()
        };

        let mut logs = docker.logs(&container_name, Some(logs_options));
        let mut output = String::new();

        while let Some(Ok(log)) = logs.next().await {
            output.push_str(&log.to_string());
        }

        docker
            .remove_container(
                &container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .ok();

        match wait_result {
            Ok(Some(Ok(result))) => {
                if result.status_code == 0 {
                    Ok(if output.is_empty() {
                        "Success".to_string()
                    } else {
                        output
                    })
                } else {
                    Err(ToolError::CommandFailed(output))
                }
            }
            Ok(Some(Err(e))) => Err(ToolError::CommandFailed(e.to_string())),
            Ok(None) => Err(ToolError::CommandFailed(
                "Container exited unexpectedly".to_string(),
            )),
            Err(_) => Err(ToolError::Timeout),
        }
    }

    fn process_output(output: std::process::Output) -> Result<String, ToolError> {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&stderr);
            }

            Ok(if result.is_empty() {
                "Success".to_string()
            } else {
                result
            })
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(ToolError::CommandFailed(stderr.to_string()))
        }
    }
}

impl Tool for RunCommand {
    const NAME: &'static str = "run_command";

    type Error = ToolError;
    type Args = RunCommandArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let (shell_name, description) = if self.is_owner {
            let shell = get_default_shell();
            let name = get_shell_name(&shell);
            (
                name.to_string(),
                format!("Execute shell commands using {} shell", name),
            )
        } else {
            ("bash".to_string(), "Execute shell commands in a sandboxed Docker container. If you need programming language runtime, use 'mise' to install them.".to_string())
        };

        ToolDefinition {
            name: Self::NAME.to_string(),
            description,
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": format!("Command to execute (shell: {})", shell_name)
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if self.is_owner {
            self.run_local(&args.command).await
        } else {
            self.run_sandbox(&args.command).await
        }
    }
}
