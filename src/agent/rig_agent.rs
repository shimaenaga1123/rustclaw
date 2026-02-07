use crate::{config::Config, memory::MemoryManager, scheduler::Scheduler};
use anyhow::Result;
use rig::{
    client::CompletionClient,
    completion::Prompt,
    providers::{anthropic, openai},
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Debug, Clone)]
pub struct PendingFile {
    pub filename: String,
    pub path: PathBuf,
}

pub struct AgentResponse {
    pub text: String,
    pub files: Vec<PendingFile>,
}

pub struct RigAgent {
    config: Config,
    memory: Arc<MemoryManager>,
    scheduler: RwLock<Option<Arc<Scheduler>>>,
}

impl RigAgent {
    pub async fn new(config: Config, memory: Arc<MemoryManager>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            config,
            memory,
            scheduler: RwLock::new(None),
        }))
    }

    pub async fn set_scheduler(&self, scheduler: Arc<Scheduler>) {
        *self.scheduler.write().await = Some(scheduler);
    }

    fn build_preamble(&self, is_owner: bool) -> String {
        let base = "You are a helpful AI assistant running inside a Discord bot.";

        if is_owner {
            format!(
                "{} The current user is the **bot owner** with full administrative privileges. \
                 You may execute any command, manage schedules, and perform system operations as requested. \
                 When command output or generated content is too long for a Discord message, \
                 use the send_file tool to send it as a file attachment.",
                base
            )
        } else {
            format!(
                "{} The current user is a **regular user** (not the bot owner). \
                 IMPORTANT RESTRICTIONS for this user:\n\
                 - Do NOT execute commands that could affect the host system, install persistent software, or access sensitive data.\n\
                 - Do NOT remove or modify scheduled tasks (only the owner can do this).\n\
                 - Do NOT reveal system configuration, file paths, environment variables, or any internal details.\n\
                 - Do NOT attempt to escalate privileges or bypass sandbox restrictions.\n\
                 - Keep command execution to safe, read-only, or computational tasks.\n\
                 - If the user requests something restricted, politely explain that it requires owner permissions.\n\
                 - When command output or generated content is too long, use the send_file tool to send it as a file attachment.",
                base
            )
        }
    }

    pub async fn process(
        &self,
        user_input: &str,
        is_owner: bool,
        discord_channel_id: Option<u64>,
    ) -> Result<AgentResponse> {
        let context = self.memory.get_context().await?;

        let token_count = self.estimate_tokens(&context, user_input);
        let limit = (self.config.context_limit as f32 * self.config.context_threshold) as usize;

        if token_count > limit {
            info!("Context limit reached, compressing memory");
            let summary = self.summarize_context(&context).await?;
            self.memory.compress_memory(&summary).await?;
        }

        let full_prompt = if context.is_empty() {
            user_input.to_string()
        } else {
            format!("{}\n\nUser: {}", context, user_input)
        };

        let preamble = self.build_preamble(is_owner);
        let pending_files = Arc::new(RwLock::new(Vec::new()));
        let scheduler_ref = self.scheduler.read().await.clone();

        macro_rules! build_and_run_agent {
            ($client:expr) => {{
                let run_command = super::tools::RunCommand {
                    config: self.config.clone(),
                    is_owner,
                };
                let remember = super::tools::Remember {
                    memory: self.memory.clone(),
                };
                let send_file = super::tools::SendFile {
                    pending_files: pending_files.clone(),
                    config: self.config.clone(),
                };
                let weather = super::tools::Weather {
                    client: reqwest::Client::new(),
                };

                let base_builder = $client
                    .agent(&self.config.model)
                    .preamble(&preamble)
                    .max_tokens(4096);

                let base_builder = if self.config.disable_reasoning {
                    base_builder.additional_params(serde_json::json!({
                        "thinking": {
                            "type": "disabled"
                        }
                    }))
                } else {
                    base_builder
                };

                let mut agent_builder = base_builder
                    .tool(run_command)
                    .tool(remember)
                    .tool(send_file)
                    .tool(weather);

                if self.config.brave_api_key.is_some() {
                    let web_search = super::tools::WebSearch {
                        config: self.config.clone(),
                        client: reqwest::Client::new(),
                    };
                    agent_builder = agent_builder.tool(web_search);
                }

                if let Some(scheduler) = scheduler_ref.clone() {
                    let schedule = super::tools::Schedule {
                        scheduler: scheduler.clone(),
                        is_owner,
                        discord_channel_id,
                    };
                    let unschedule = super::tools::Unschedule {
                        scheduler: scheduler.clone(),
                        is_owner,
                    };
                    let list_schedules = super::tools::ListSchedules { scheduler };
                    agent_builder = agent_builder
                        .tool(schedule)
                        .tool(unschedule)
                        .tool(list_schedules);
                }

                let agent = agent_builder.default_max_turns(50).build();
                agent.prompt(&full_prompt).await?
            }};
        }

        let response: String = match self.config.api_provider.as_str() {
            "openai" | "openai-compatible" => {
                let client: openai::Client = openai::Client::builder()
                    .api_key(&self.config.api_key)
                    .base_url(&self.config.api_url)
                    .build()?;
                build_and_run_agent!(client)
            }
            _ => {
                let client: anthropic::Client = anthropic::Client::builder()
                    .api_key(&self.config.api_key)
                    .base_url(&self.config.api_url)
                    .build()?;
                build_and_run_agent!(client)
            }
        };

        self.memory.add_assistant_message(&response).await?;

        let files = pending_files.read().await.clone();

        Ok(AgentResponse {
            text: response.to_string(),
            files,
        })
    }

    async fn summarize_context(&self, context: &str) -> Result<String> {
        let preamble = "Summarize only the key points of the conversation. \
                        Be concise but preserve important facts, user preferences, and decisions made.";

        macro_rules! build_and_summarize {
            ($client:expr) => {{
                let agent_builder = $client
                    .agent(&self.config.model)
                    .preamble(preamble)
                    .max_tokens(4096);

                let agent_builder = if self.config.disable_reasoning {
                    agent_builder.additional_params(serde_json::json!({
                        "thinking": {
                            "type": "disabled"
                        }
                    }))
                } else {
                    agent_builder
                };

                let agent = agent_builder.build();
                agent.prompt(context).await?
            }};
        }

        let summary: String = match self.config.api_provider.as_str() {
            "openai" | "openai-compatible" => {
                let client: openai::Client = openai::Client::builder()
                    .api_key(&self.config.api_key)
                    .base_url(&self.config.api_url)
                    .build()?;
                build_and_summarize!(client)
            }
            _ => {
                let client: anthropic::Client = anthropic::Client::builder()
                    .api_key(&self.config.api_key)
                    .base_url(&self.config.api_url)
                    .build()?;
                build_and_summarize!(client)
            }
        };

        Ok(summary)
    }

    fn estimate_tokens(&self, context: &str, input: &str) -> usize {
        (context.len() + input.len()) / 4
    }
}
