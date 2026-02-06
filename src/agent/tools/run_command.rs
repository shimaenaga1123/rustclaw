use super::error::ToolError;
use crate::config::Config;
use bollard::{
    Docker,
    container::{
        Config as ContainerConfig, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
        StartContainerOptions, WaitContainerOptions,
    },
    image::CreateImageOptions,
};
use futures_util::StreamExt;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tracing::{debug, info};

const SANDBOX_IMAGE: &str = "alpine:latest";

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
    async fn ensure_image(docker: &Docker) -> Result<(), ToolError> {
        if docker.inspect_image(SANDBOX_IMAGE).await.is_ok() {
            return Ok(());
        }

        info!("Pulling sandbox image: {}", SANDBOX_IMAGE);
        let options = CreateImageOptions {
            from_image: SANDBOX_IMAGE,
            ..Default::default()
        };

        let mut stream = docker.create_image(Some(options), None, None);
        while let Some(result) = stream.next().await {
            result.map_err(|e| ToolError::CommandFailed(format!("Image pull failed: {}", e)))?;
        }

        Ok(())
    }

    async fn run_in_container(&self, command: &str) -> Result<String, ToolError> {
        debug!(
            "Executing in Alpine container (owner={}): {}",
            self.is_owner, command
        );

        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| ToolError::CommandFailed(format!("Docker connection failed: {}", e)))?;

        Self::ensure_image(&docker).await?;

        let container_name = format!("sandbox-{}", uuid::Uuid::new_v4());

        // Non-owner: disable network access
        let network_disabled = !self.is_owner;

        let config = ContainerConfig {
            image: Some(SANDBOX_IMAGE),
            cmd: Some(vec!["sh", "-c", command]),
            working_dir: Some("/workspace"),
            network_disabled: Some(network_disabled),
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

        let timeout_secs = if self.is_owner {
            self.config.command_timeout
        } else {
            self.config.command_timeout.min(15)
        };

        let wait_result = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
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

        if !self.is_owner && output.len() > 4096 {
            output.truncate(4096);
            output.push_str("\n... (output truncated)");
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
}

impl Tool for RunCommand {
    const NAME: &'static str = "run_command";

    type Error = ToolError;
    type Args = RunCommandArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Execute shell commands in an isolated Alpine Linux Docker container. Use 'apk add' to install packages."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command to execute in Alpine Linux container (shell: sh)"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.run_in_container(&args.command).await
    }
}
