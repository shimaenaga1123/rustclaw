use super::error::ToolError;
use crate::config::Config;
use bollard::{
    Docker,
    container::{
        Config as ContainerConfig, CreateContainerOptions, LogsOptions, RemoveContainerOptions,
        StartContainerOptions, StopContainerOptions, WaitContainerOptions,
    },
    image::CreateImageOptions,
    models::HostConfig,
};
use futures_util::StreamExt;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, info, warn};

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
    pub fn workspace_path(config: &Config) -> PathBuf {
        config.data_dir.join("workspace")
    }

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

    async fn collect_logs(docker: &Docker, container_name: &str) -> String {
        let logs_options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            ..Default::default()
        };

        let mut logs = docker.logs(container_name, Some(logs_options));
        let mut output = String::new();

        while let Some(Ok(log)) = logs.next().await {
            output.push_str(&log.to_string());
        }

        output
    }

    async fn cleanup_container(docker: &Docker, container_name: &str) {
        docker
            .remove_container(
                container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .ok();
    }

    async fn run_in_container(&self, command: &str) -> Result<String, ToolError> {
        debug!(
            "Executing in Alpine container (owner={}): {}",
            self.is_owner, command
        );

        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| ToolError::CommandFailed(format!("Docker connection failed: {}", e)))?;

        Self::ensure_image(&docker).await?;

        let workspace = Self::workspace_path(&self.config);
        tokio::fs::create_dir_all(&workspace)
            .await
            .map_err(|e| ToolError::CommandFailed(format!("Failed to create workspace: {}", e)))?;

        let workspace_abs = workspace.canonicalize().map_err(|e| {
            ToolError::CommandFailed(format!("Failed to resolve workspace path: {}", e))
        })?;

        let container_name = format!("sandbox-{}", uuid::Uuid::new_v4());
        let network_disabled = !self.is_owner;

        let bind = format!("{}:/workspace", workspace_abs.display());

        let host_config = HostConfig {
            binds: Some(vec![bind]),
            ..Default::default()
        };

        let config = ContainerConfig {
            image: Some(SANDBOX_IMAGE),
            cmd: Some(vec!["sh", "-c", command]),
            working_dir: Some("/workspace"),
            network_disabled: Some(network_disabled),
            host_config: Some(host_config),
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
            .map_err(|e| {
                let docker = docker.clone();
                let name = container_name.clone();
                tokio::spawn(async move { Self::cleanup_container(&docker, &name).await });
                ToolError::CommandFailed(format!("Container start failed: {}", e))
            })?;

        let timeout_secs = if self.is_owner {
            self.config.command_timeout
        } else {
            self.config.command_timeout.min(15)
        };

        let wait_options = WaitContainerOptions {
            condition: "not-running",
        };

        let exit_code = match tokio::time::timeout(Duration::from_secs(timeout_secs), async {
            let mut stream = docker.wait_container(&container_name, Some(wait_options));
            stream.next().await
        })
        .await
        {
            Ok(Some(Ok(response))) => Some(response.status_code),
            Ok(Some(Err(e))) => {
                warn!(
                    "Container wait returned error (will still collect logs): {}",
                    e
                );
                docker
                    .inspect_container(&container_name, None)
                    .await
                    .ok()
                    .and_then(|info| info.state)
                    .and_then(|s| s.exit_code)
            }
            Ok(None) => {
                warn!("Container wait stream ended without result");
                docker
                    .inspect_container(&container_name, None)
                    .await
                    .ok()
                    .and_then(|info| info.state)
                    .and_then(|s| s.exit_code)
            }
            Err(_) => {
                warn!("Container execution timed out after {}s", timeout_secs);
                docker
                    .stop_container(&container_name, Some(StopContainerOptions { t: 2 }))
                    .await
                    .ok();

                let output = Self::collect_logs(&docker, &container_name).await;
                Self::cleanup_container(&docker, &container_name).await;

                if output.is_empty() {
                    return Err(ToolError::Timeout);
                } else {
                    return Err(ToolError::CommandFailed(format!(
                        "(timed out after {}s)\n{}",
                        timeout_secs, output
                    )));
                }
            }
        };

        let mut output = Self::collect_logs(&docker, &container_name).await;
        Self::cleanup_container(&docker, &container_name).await;

        if !self.is_owner && output.len() > 4096 {
            output.truncate(4096);
            output.push_str("\n... (output truncated)");
        }

        match exit_code {
            Some(0) => Ok(if output.is_empty() {
                "Success".to_string()
            } else {
                output
            }),
            Some(code) => {
                if output.is_empty() {
                    Err(ToolError::CommandFailed(format!(
                        "Command exited with code {}",
                        code
                    )))
                } else {
                    Err(ToolError::CommandFailed(format!(
                        "(exit code: {})\n{}",
                        code, output
                    )))
                }
            }
            None => {
                if output.is_empty() {
                    Err(ToolError::CommandFailed(
                        "Command finished with unknown status and no output".to_string(),
                    ))
                } else {
                    Ok(format!("(exit code unknown)\n{}", output))
                }
            }
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
            description:
                "Execute shell commands in an isolated Alpine Linux Docker container. \
                 The /workspace directory is shared between commands — files written there persist across invocations. \
                 Use 'apk add' to install packages."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command to execute in Alpine Linux container (shell: sh). Files saved to /workspace persist."
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
