use crate::{config::Config, memory::MemoryManager, scheduler::Scheduler};
use anyhow::Result;
use futures_util::StreamExt;
use rig::{
    agent::MultiTurnStreamItem,
    client::CompletionClient,
    completion::{CompletionModel, GetTokenUsage, Prompt},
    providers::{anthropic, gemini, openai},
    streaming::{StreamedAssistantContent, StreamingPrompt},
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, mpsc};
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

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    Done,
    Error(String),
}

#[derive(Debug, Clone, Default)]
pub struct UserInfo {
    pub name: String,
    pub global_name: Option<String>,
    pub nickname: Option<String>,
    pub id: u64,
    pub roles: Vec<String>,
    pub avatar_url: Option<String>,
}

impl UserInfo {
    pub fn format_for_prompt(&self) -> String {
        let mut parts = vec![format!("Username: {}", self.name)];

        if let Some(ref gn) = self.global_name {
            parts.push(format!("Display name: {}", gn));
        }
        if let Some(ref nick) = self.nickname {
            parts.push(format!("Server nickname: {}", nick));
        }

        parts.push(format!("ID: {}", self.id));

        if !self.roles.is_empty() {
            parts.push(format!("Roles: {}", self.roles.join(", ")));
        }
        if let Some(ref url) = self.avatar_url {
            parts.push(format!("Avatar: {}", url));
        }

        format!("[User Info]\n{}", parts.join("\n"))
    }
}

#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    pub filename: String,
    pub container_path: String,
    pub size: u32,
    pub content_type: Option<String>,
}

impl AttachmentInfo {
    pub fn format_for_prompt(attachments: &[AttachmentInfo]) -> String {
        if attachments.is_empty() {
            return String::new();
        }

        let mut out = String::from("[Attachments uploaded to /workspace/upload/]\n");
        for att in attachments {
            out.push_str(&format!(
                "- {} ({} bytes, {}): {}\n",
                att.filename,
                att.size,
                att.content_type.as_deref().unwrap_or("unknown"),
                att.container_path,
            ));
        }
        out
    }
}

enum ApiClient {
    Anthropic(anthropic::Client),
    OpenAi(openai::CompletionsClient),
    Gemini(gemini::Client),
}

struct StreamParams<'a> {
    model: &'a str,
    preamble: &'a str,
    prompt: &'a str,
    disable_reasoning: bool,
    is_owner: bool,
    discord_channel_id: Option<u64>,
    config: &'a Config,
    memory: &'a Arc<MemoryManager>,
    scheduler: Option<Arc<Scheduler>>,
    pending_files: Arc<RwLock<Vec<PendingFile>>>,
    tx: mpsc::Sender<StreamEvent>,
}

impl ApiClient {
    async fn prompt(
        &self,
        model: &str,
        preamble: &str,
        prompt: &str,
        disable_reasoning: bool,
    ) -> Result<String> {
        match self {
            ApiClient::OpenAi(client) => {
                let builder = client.agent(model).preamble(preamble).max_tokens(4096);
                let builder = if disable_reasoning {
                    builder.additional_params(serde_json::json!({"thinking": {"type": "disabled"}}))
                } else {
                    builder
                };
                Ok(builder.build().prompt(prompt).await?)
            }
            ApiClient::Gemini(client) => {
                let builder = client.agent(model).preamble(preamble).max_tokens(4096);
                let builder = if disable_reasoning {
                    builder.additional_params(serde_json::json!({"thinking": {"type": "disabled"}}))
                } else {
                    builder
                };
                Ok(builder.build().prompt(prompt).await?)
            }
            ApiClient::Anthropic(client) => {
                let builder = client.agent(model).preamble(preamble).max_tokens(4096);
                let builder = if disable_reasoning {
                    builder.additional_params(serde_json::json!({"thinking": {"type": "disabled"}}))
                } else {
                    builder
                };
                Ok(builder.build().prompt(prompt).await?)
            }
        }
    }

    async fn stream_prompt(&self, params: StreamParams<'_>) -> Result<String> {
        let run_command = super::tools::RunCommand {
            config: params.config.clone(),
            is_owner: params.is_owner,
        };
        let remember = super::tools::Remember {
            memory: params.memory.clone(),
        };
        let send_file = super::tools::SendFile {
            pending_files: params.pending_files.clone(),
            config: params.config.clone(),
        };
        let send_markdown_table = super::tools::SendMarkdownTable {
            pending_files: params.pending_files.clone(),
            config: params.config.clone(),
        };
        let weather = super::tools::Weather {
            client: reqwest::Client::new(),
        };

        match self {
            ApiClient::OpenAi(client) => {
                let builder = client
                    .agent(params.model)
                    .preamble(params.preamble)
                    .max_tokens(4096);
                let builder = if params.disable_reasoning {
                    builder.additional_params(serde_json::json!({"thinking": {"type": "disabled"}}))
                } else {
                    builder
                };
                let mut builder = builder
                    .tool(run_command)
                    .tool(remember)
                    .tool(send_file)
                    .tool(send_markdown_table.clone())
                    .tool(weather);

                if params.is_owner {
                    builder = builder.tool(super::tools::ResetContainer {
                        config: params.config.clone(),
                    });
                }
                if params.config.brave_api_key.is_some() {
                    builder = builder.tool(super::tools::WebSearch {
                        config: params.config.clone(),
                        client: reqwest::Client::new(),
                    });
                }
                if let Some(ref scheduler) = params.scheduler {
                    builder = builder
                        .tool(super::tools::Schedule {
                            scheduler: scheduler.clone(),
                            is_owner: params.is_owner,
                            discord_channel_id: params.discord_channel_id,
                        })
                        .tool(super::tools::Unschedule {
                            scheduler: scheduler.clone(),
                            is_owner: params.is_owner,
                        })
                        .tool(super::tools::ListSchedules {
                            scheduler: scheduler.clone(),
                        });
                }

                Self::run_stream(
                    builder.default_max_turns(50).build(),
                    params.prompt,
                    params.tx,
                )
                .await
            }
            ApiClient::Gemini(client) => {
                let builder = client
                    .agent(params.model)
                    .preamble(params.preamble)
                    .max_tokens(4096);
                let builder = if params.disable_reasoning {
                    builder.additional_params(serde_json::json!({"thinking": {"type": "disabled"}}))
                } else {
                    builder
                };
                let mut builder = builder
                    .tool(run_command)
                    .tool(remember)
                    .tool(send_file)
                    .tool(send_markdown_table.clone())
                    .tool(weather);

                if params.is_owner {
                    builder = builder.tool(super::tools::ResetContainer {
                        config: params.config.clone(),
                    });
                }
                if params.config.brave_api_key.is_some() {
                    builder = builder.tool(super::tools::WebSearch {
                        config: params.config.clone(),
                        client: reqwest::Client::new(),
                    });
                }
                if let Some(ref scheduler) = params.scheduler {
                    builder = builder
                        .tool(super::tools::Schedule {
                            scheduler: scheduler.clone(),
                            is_owner: params.is_owner,
                            discord_channel_id: params.discord_channel_id,
                        })
                        .tool(super::tools::Unschedule {
                            scheduler: scheduler.clone(),
                            is_owner: params.is_owner,
                        })
                        .tool(super::tools::ListSchedules {
                            scheduler: scheduler.clone(),
                        });
                }

                Self::run_stream(
                    builder.default_max_turns(50).build(),
                    params.prompt,
                    params.tx,
                )
                .await
            }
            ApiClient::Anthropic(client) => {
                let builder = client
                    .agent(params.model)
                    .preamble(params.preamble)
                    .max_tokens(4096);
                let builder = if params.disable_reasoning {
                    builder.additional_params(serde_json::json!({"thinking": {"type": "disabled"}}))
                } else {
                    builder
                };
                let mut builder = builder
                    .tool(run_command)
                    .tool(remember)
                    .tool(send_file)
                    .tool(send_markdown_table)
                    .tool(weather);

                if params.is_owner {
                    builder = builder.tool(super::tools::ResetContainer {
                        config: params.config.clone(),
                    });
                }
                if params.config.brave_api_key.is_some() {
                    builder = builder.tool(super::tools::WebSearch {
                        config: params.config.clone(),
                        client: reqwest::Client::new(),
                    });
                }
                if let Some(ref scheduler) = params.scheduler {
                    builder = builder
                        .tool(super::tools::Schedule {
                            scheduler: scheduler.clone(),
                            is_owner: params.is_owner,
                            discord_channel_id: params.discord_channel_id,
                        })
                        .tool(super::tools::Unschedule {
                            scheduler: scheduler.clone(),
                            is_owner: params.is_owner,
                        })
                        .tool(super::tools::ListSchedules {
                            scheduler: scheduler.clone(),
                        });
                }

                Self::run_stream(
                    builder.default_max_turns(50).build(),
                    params.prompt,
                    params.tx,
                )
                .await
            }
        }
    }

    async fn run_stream<M, R, A>(
        agent: A,
        prompt: &str,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<String>
    where
        M: CompletionModel + 'static,
        R: Clone + Unpin + GetTokenUsage,
        A: StreamingPrompt<M, R>,
    {
        let mut stream = agent.stream_prompt(prompt).await;
        let mut response_text = String::new();

        while let Some(item) = stream.next().await {
            match item {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    text,
                ))) => {
                    let _ = tx.send(StreamEvent::TextDelta(text.text.clone())).await;
                    response_text.push_str(&text.text);
                }
                Ok(MultiTurnStreamItem::FinalResponse(res)) => {
                    if response_text.is_empty() {
                        response_text = res.response().to_string();
                    }
                }
                Err(e) => {
                    let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                    return Err(anyhow::anyhow!("{}", e));
                }
                _ => {}
            }
        }

        let _ = tx.send(StreamEvent::Done).await;
        Ok(response_text)
    }
}

pub struct RigAgent {
    config: Config,
    memory: Arc<MemoryManager>,
    scheduler: RwLock<Option<Arc<Scheduler>>>,
    client: ApiClient,
    compress_lock: Mutex<()>,
}

impl RigAgent {
    pub async fn new(config: Config, memory: Arc<MemoryManager>) -> Result<Arc<Self>> {
        let client = match config.api_provider.as_str() {
            "openai" | "openai-compatible" => ApiClient::OpenAi(
                openai::CompletionsClient::builder()
                    .api_key(&config.api_key)
                    .base_url(&config.api_url)
                    .build()?,
            ),
            "gemini" => ApiClient::Gemini(gemini::Client::new(&config.api_key)?),
            _ => ApiClient::Anthropic(
                anthropic::Client::builder()
                    .api_key(&config.api_key)
                    .base_url(&config.api_url)
                    .build()?,
            ),
        };

        Ok(Arc::new(Self {
            config,
            memory,
            scheduler: RwLock::new(None),
            client,
            compress_lock: Mutex::new(()),
        }))
    }

    pub async fn set_scheduler(&self, scheduler: Arc<Scheduler>) {
        *self.scheduler.write().await = Some(scheduler);
    }

    fn build_preamble(&self, is_owner: bool) -> String {
        let now = chrono::Local::now();
        let timezone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_string());

        let mut preamble = format!(
            "You are a helpful AI assistant running inside a Discord bot called RustClaw.\n\
         Current time: {} ({})\n\n",
            now.format("%Y-%m-%d %H:%M:%S"),
            timezone
        );

        // Identity & behavior
        preamble.push_str(
            "# Behavior\n\
         - Be concise and direct. Discord messages have a 2000-character limit, so avoid unnecessary verbosity.\n\
         - Use Discord markdown for formatting: **bold**, *italic*, `inline code`, ```code blocks```, > quotes.\n\
         - Do NOT use headers (#), horizontal rules (---), or HTML tags — they don't render in Discord.\n\
         - Respond in the same language the user writes in.\n\
         - When a task involves multiple steps, execute them sequentially without asking for confirmation at each step.\n\n"
        );

        // Tool usage guidelines
        preamble.push_str(
            "# Tool Usage\n\
         - **run_command**: Runs shell commands in a persistent Debian Docker container at /workspace. \
           Bun and Node.js are pre-installed. Installed packages persist across invocations. \
           For Python, install it first with `apt-get install -y python3`.\n\
         - **send_file**: Sends a file from /workspace as a Discord attachment. \
           Use this when output is too long for a message, or when generating files (images, documents, code, etc.).\n\
         - **send_markdown_table**: Renders markdown tables as images and sends them as Discord attachments. \
           Use this when you need to display tables, since Discord doesn't support markdown table formatting.\n\
         - **remember**: Saves important facts to long-term memory. Use proactively when the user shares \
           personal preferences, important dates, project details, or anything worth recalling later.\n\
         - **web_search**: Searches the web via Brave Search. Use for current events, fact-checking, or \
           looking up information you're unsure about.\n\
         - **weather**: Gets current weather and forecasts. Use when the user asks about weather.\n\
         - **schedule / list_schedules / unschedule**: Manages cron-based recurring tasks.\n\n"
        );

        // Attachments
        preamble.push_str(
            "# Attachments\n\
         User-uploaded files are saved to /workspace/upload/ in the container. \
         You can read, process, convert, or analyze them using run_command. \
         The prompt will include an [Attachments] section listing filenames, sizes, and paths when files are present.\n\n"
        );

        // Memory
        preamble.push_str(
            "# Memory\n\
         You have access to short-term conversation history and long-term memory. \
         Long-term memory entries appear at the top of the context under '# Long-term Memory'. \
         Use the remember tool to save new important information. \
         Avoid storing duplicate or trivial information.\n\n",
        );

        // Permission level
        if is_owner {
            preamble.push_str(
                "# Permissions\n\
             The current user is the **bot owner** with full administrative privileges.\n\
             - You may execute any command, including system-level operations.\n\
             - You may manage all scheduled tasks (create, list, remove).\n\
             - You may reset the Docker container when needed.\n\
             - No output restrictions apply.\n",
            );
        } else {
            preamble.push_str(
                "# Permissions\n\
             The current user is a **regular user** (not the bot owner).\n\
             - Do NOT execute commands that could affect the host system, install persistent software, or access sensitive data.\n\
             - Do NOT remove or modify scheduled tasks (only the owner can do this).\n\
             - Do NOT reveal system configuration, file paths outside /workspace, environment variables, or internal details.\n\
             - Do NOT attempt to escalate privileges or bypass sandbox restrictions.\n\
             - Keep command execution to safe, read-only, or computational tasks.\n\
             - If a request requires owner permissions, politely explain the restriction.\n"
            );
        }
        preamble
    }

    pub async fn process_streaming(
        &self,
        user_input: &str,
        is_owner: bool,
        discord_channel_id: Option<u64>,
        user_info: Option<&UserInfo>,
        attachments: &[AttachmentInfo],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<AgentResponse> {
        let context = self.memory.get_context().await?;

        let token_count = self.estimate_tokens(&context, user_input);
        let limit = (self.config.context_limit as f32 * self.config.context_threshold) as usize;

        if token_count > limit {
            let _guard = self.compress_lock.lock().await;
            let context = self.memory.get_context().await?;
            let token_count = self.estimate_tokens(&context, user_input);
            if token_count > limit {
                info!("Context limit reached, compressing memory");
                let summary = self.summarize_context(&context).await?;
                self.memory.compress_memory(&summary).await?;
            }
        }

        let context = self.memory.get_context().await?;
        let user_section = user_info.map(|u| u.format_for_prompt()).unwrap_or_default();
        let attachment_section = AttachmentInfo::format_for_prompt(attachments);

        let mut full_prompt = String::new();
        if !context.is_empty() {
            full_prompt.push_str(&context);
            full_prompt.push_str("\n\n");
        }
        if !user_section.is_empty() {
            full_prompt.push_str(&user_section);
            full_prompt.push('\n');
        }
        if !attachment_section.is_empty() {
            full_prompt.push_str(&attachment_section);
            full_prompt.push('\n');
        }
        full_prompt.push_str(&format!("User: {}", user_input));

        let preamble = self.build_preamble(is_owner);
        let pending_files = Arc::new(RwLock::new(Vec::new()));
        let scheduler_ref = self.scheduler.read().await.clone();

        let response = self
            .client
            .stream_prompt(StreamParams {
                model: &self.config.model,
                preamble: &preamble,
                prompt: &full_prompt,
                disable_reasoning: self.config.disable_reasoning,
                is_owner,
                discord_channel_id,
                config: &self.config,
                memory: &self.memory,
                scheduler: scheduler_ref,
                pending_files: pending_files.clone(),
                tx,
            })
            .await?;

        self.memory.add_assistant_message(&response).await?;
        let files = pending_files.read().await.clone();

        Ok(AgentResponse {
            text: response,
            files,
        })
    }

    async fn summarize_context(&self, context: &str) -> Result<String> {
        let preamble = "Summarize only the key points of the conversation. \
                        Be concise but preserve important facts, user preferences, and decisions made.";

        self.client
            .prompt(
                &self.config.model,
                preamble,
                context,
                self.config.disable_reasoning,
            )
            .await
    }

    fn estimate_tokens(&self, context: &str, input: &str) -> usize {
        (context.chars().count() + input.chars().count()) / 3
    }
}
