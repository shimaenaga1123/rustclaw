use super::error::ToolError;
use crate::config::Config;
use bollard::{
    Docker,
    exec::{CreateExecOptions, StartExecResults},
    models::{ContainerCreateBody, HostConfig},
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, RemoveContainerOptionsBuilder,
        StopContainerOptionsBuilder,
    },
};
use futures_util::StreamExt;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

const SANDBOX_IMAGE: &str = "oven/bun:debian";
const CONTAINER_NAME: &str = "rustclaw-sandbox";

static CONTAINER_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

#[derive(Deserialize, Serialize)]
pub struct RunCommandArgs {
    pub command: String,
}

#[derive(Clone)]
pub struct RunCommand {
    pub config: Arc<Config>,
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
        let options = CreateImageOptionsBuilder::new()
            .from_image(SANDBOX_IMAGE)
            .build();

        let mut stream = docker.create_image(Some(options), None, None);
        while let Some(result) = stream.next().await {
            result.map_err(|e| ToolError::CommandFailed(format!("Image pull failed: {}", e)))?;
        }

        Ok(())
    }

    async fn ensure_container(docker: &Docker) -> Result<(), ToolError> {
        let _lock = CONTAINER_LOCK.lock().await;

        match docker.inspect_container(CONTAINER_NAME, None).await {
            Ok(info) => {
                let running = info.state.as_ref().and_then(|s| s.running).unwrap_or(false);

                if !running {
                    info!("Sandbox container exists but not running, starting...");
                    docker
                        .start_container(CONTAINER_NAME, None)
                        .await
                        .map_err(|e| {
                            ToolError::CommandFailed(format!("Container start failed: {}", e))
                        })?;
                }

                Ok(())
            }
            Err(_) => {
                info!("Creating persistent sandbox container: {}", CONTAINER_NAME);

                Self::ensure_image(docker).await?;

                let volume_name = "rustclaw-workspace";
                let bind = format!("{}:/workspace", volume_name);

                let host_config = HostConfig {
                    binds: Some(vec![bind]),
                    ..Default::default()
                };

                let container_config = ContainerCreateBody {
                    image: Some(SANDBOX_IMAGE.to_string()),
                    cmd: Some(vec!["sleep".to_string(), "infinity".to_string()]),
                    working_dir: Some("/workspace".to_string()),
                    network_disabled: Some(false),
                    host_config: Some(host_config),
                    tty: Some(true),
                    ..Default::default()
                };

                let options = CreateContainerOptionsBuilder::new()
                    .name(CONTAINER_NAME)
                    .build();

                docker
                    .create_container(Some(options), container_config)
                    .await
                    .map_err(|e| {
                        ToolError::CommandFailed(format!("Container creation failed: {}", e))
                    })?;

                docker
                    .start_container(CONTAINER_NAME, None)
                    .await
                    .map_err(|e| {
                        ToolError::CommandFailed(format!("Container start failed: {}", e))
                    })?;

                info!("Persistent sandbox container started");
                Ok(())
            }
        }
    }

    pub async fn reset_container(config: &Config) -> Result<(), ToolError> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| ToolError::CommandFailed(format!("Docker connection failed: {}", e)))?;

        let _lock = CONTAINER_LOCK.lock().await;

        docker
            .stop_container(
                CONTAINER_NAME,
                Some(StopContainerOptionsBuilder::new().t(2).build()),
            )
            .await
            .ok();

        docker
            .remove_container(
                CONTAINER_NAME,
                Some(RemoveContainerOptionsBuilder::new().force(true).build()),
            )
            .await
            .ok();

        let workspace = Self::workspace_path(config);
        if workspace.exists() {
            tokio::fs::remove_dir_all(&workspace).await.ok();
        }

        info!("Sandbox container reset");
        Ok(())
    }

    async fn exec_in_container(&self, command: &str) -> Result<String, ToolError> {
        debug!("Executing in persistent container: {}", command);

        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| ToolError::CommandFailed(format!("Docker connection failed: {}", e)))?;

        Self::ensure_container(&docker).await?;

        let exec_options = CreateExecOptions {
            cmd: Some(vec!["bash", "-c", command]),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            working_dir: Some("/workspace"),
            ..Default::default()
        };

        let exec = docker
            .create_exec(CONTAINER_NAME, exec_options)
            .await
            .map_err(|e| ToolError::CommandFailed(format!("Exec creation failed: {}", e)))?;

        let exec_id = exec.id.clone();

        match tokio::time::timeout(Duration::from_secs(self.config.command_timeout), async {
            let start_result = docker
                .start_exec(&exec.id, None)
                .await
                .map_err(|e| ToolError::CommandFailed(format!("Exec start failed: {}", e)))?;

            let mut output = String::new();

            match start_result {
                StartExecResults::Attached {
                    output: mut stream, ..
                } => {
                    while let Some(Ok(msg)) = stream.next().await {
                        output.push_str(&msg.to_string());
                    }
                }
                StartExecResults::Detached => {}
            }

            Ok::<String, ToolError>(output)
        })
        .await
        {
            Ok(Ok(output)) => {
                let inspect = docker.inspect_exec(&exec_id).await.ok();
                let exit_code = inspect.and_then(|i| i.exit_code);

                match exit_code {
                    Some(0) | None => Ok(if output.is_empty() {
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
                }
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                warn!(
                    "Command execution timed out after {}s",
                    self.config.command_timeout
                );
                Err(ToolError::Timeout)
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
            description: "Execute shell commands in a persistent Debian Docker container with Bun runtime. \
                 Installed packages (apt-get install) and files persist across invocations. \
                 The /workspace directory is the working directory. Bun is pre-installed. Use bun as Node.js runtime and package manager."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command to execute in Debian container with Bun (shell: bash). Installed packages and files persist."
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.exec_in_container(&args.command).await
    }
}

#[derive(Deserialize, Serialize)]
pub struct ResetContainerArgs {}

#[derive(Clone)]
pub struct ResetContainer {
    pub config: Arc<Config>,
}

impl Tool for ResetContainer {
    const NAME: &'static str = "reset_container";

    type Error = ToolError;
    type Args = ResetContainerArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Reset the Docker sandbox container. This stops and removes the current container, \
                 clears the workspace directory, and allows a fresh container to be created on the next command. \
                 Use this when the container is in a broken state or needs a clean restart."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        RunCommand::reset_container(&self.config).await?;
        Ok(
            "Container reset successfully. A new container will be created on the next command."
                .to_string(),
        )
    }
}
