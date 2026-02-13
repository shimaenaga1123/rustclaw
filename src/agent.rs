use crate::{config::Config, memory::MemoryManager, scheduler::Scheduler};
use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;
use rig::{
    agent::MultiTurnStreamItem,
    client::CompletionClient,
    completion::{CompletionModel, GetTokenUsage},
    providers::{anthropic, gemini, openai},
    streaming::{StreamedAssistantContent, StreamingPrompt},
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

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

#[async_trait]
pub trait Agent: Send + Sync {
    async fn set_scheduler(&self, scheduler: Arc<Scheduler>);

    async fn process_streaming(
        &self,
        user_input: &str,
        is_owner: bool,
        discord_channel_id: Option<u64>,
        user_info: Option<&UserInfo>,
        attachments: &[AttachmentInfo],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<AgentResponse>;
}

pub struct RigAgent<C: CompletionClient> {
    config: Config,
    memory: Arc<MemoryManager>,
    scheduler: RwLock<Option<Arc<Scheduler>>>,
    client: C,
}

impl<C: CompletionClient> RigAgent<C> {
    pub async fn new(config: Config, memory: Arc<MemoryManager>, client: C) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            config,
            memory,
            scheduler: RwLock::new(None),
            client,
        }))
    }

    fn build_preamble(&self, is_owner: bool) -> String {
        let now = chrono::Local::now();
        let timezone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_string());

        let mut preamble = format!(
            "You are RustClaw, an AI assistant running as a Discord bot.\n\
             Current time: {} ({})\n\n",
            now.format("%Y-%m-%d %H:%M:%S"),
            timezone
        );

        // Behavior
        preamble.push_str(
            "# Behavior\n\
             - Use Discord markdown: # Header, **bold**, *italic*, `code`, ```codeblock```, > quote.\n\
             - Do NOT use ---, or HTML — they don't render in Discord.\n\
             - Match the user's language.\n\
             - Execute multi-step tasks sequentially without asking confirmation at each step.\n\n",
        );

        // Tools
        preamble.push_str(
            "# Tools\n\
             - **run_command**: Execute shell commands in a persistent Debian Docker container at /workspace. \
               Bun pre-installed. Installed packages persist. \
               For Python: `apt-get install -y python3`.\n\
             - **send_file**: Send a file from /workspace as a Discord attachment (max 8MB). \
               Create the file first with run_command.\n\
             - **typst_render**: Render Typst markup to PNG. Use for tables, math, or formatted content \
               that Discord markdown can't display.\n\
             - **search_memory**: Semantic search over past conversations. Use when the user asks about \
               previous discussions or you need context beyond what's already in the prompt.\n\
             - **important_add**: Save a key fact to persistent memory. Use proactively when the user shares \
               preferences, dates, project details, or decisions worth recalling later.\n\
             - **important_list**: List all saved important facts.\n\
             - **important_delete**: Delete an important fact by ID.\n\
             - **web_search**: Search the web via Brave Search for current events or fact-checking.\n\
             - **weather**: Get current weather and forecast for a location.\n\
             - **schedule**: Create a recurring cron task. **list_schedules**: List all tasks. \
               **unschedule**: Remove a task by ID.\n",
        );

        if is_owner {
            preamble.push_str(
                "- **reset_container**: Stop, remove, and recreate the Docker sandbox from scratch.\n",
            );
        }

        // Attachments
        preamble.push_str(
            "\n# Attachments\n\
             User uploads are saved to /workspace/upload/ in the container. \
             An [Attachments] section lists filenames, sizes, and paths when present. \
             Process them with run_command.\n\n",
        );

        // Memory context
        preamble.push_str(
            "# Memory\n\
             - **Important Facts**: Key facts appear under '# Important Facts' in the prompt.\n\
             - **Recent Conversations**: The last 5 turns are included for continuity.\n\
             - **Related Past Conversations**: Semantically similar past turns are auto-retrieved.\n\
             Use search_memory for deeper recall. Use important_add proactively for owner's key info.\n\n",
        );

        // Permissions
        if is_owner {
            preamble.push_str(
                "# Permissions: Owner\n\
                 Full administrative access. No restrictions on commands, schedules, or output.\n",
            );
        } else {
            preamble.push_str(
                "# Permissions: Regular User\n\
                 - Only safe, read-only, or computational commands. No system-level operations.\n\
                 - Cannot manage schedules, important facts, or reset the container.\n\
                 - Do not reveal system config, env vars, internal paths, or file contents outside /workspace.\n\
                 - Do not attempt privilege escalation. Politely decline restricted requests.\n",
            );
        }
        preamble
    }

    async fn stream_prompt(&self, params: StreamParams<'_>) -> Result<String>
    where
        <C as CompletionClient>::CompletionModel: 'static,
    {
        let mut builder = self
            .client
            .agent(params.model)
            .preamble(params.preamble)
            .tool(super::tools::RunCommand {
                config: params.config.clone(),
                is_owner: params.is_owner,
            })
            .tool(super::tools::ImportantAdd {
                vectordb: params.memory.vector_db().clone(),
                is_owner: params.is_owner,
            })
            .tool(super::tools::ImportantList {
                vectordb: params.memory.vector_db().clone(),
            })
            .tool(super::tools::ImportantDelete {
                vectordb: params.memory.vector_db().clone(),
                is_owner: params.is_owner,
            })
            .tool(super::tools::SendFile {
                pending_files: params.pending_files.clone(),
                config: params.config.clone(),
            })
            .tool(super::tools::TypstRender {
                pending_files: params.pending_files.clone(),
                config: params.config.clone(),
            })
            .tool(super::tools::SearchMemory {
                vectordb: params.memory.vector_db().clone(),
            })
            .tool(super::tools::Weather {
                client: reqwest::Client::new(),
            });

        {
            let mut extra = serde_json::Map::new();
            if params.disable_reasoning {
                extra.insert("thinking".into(), serde_json::json!({"type": "disabled"}));
            }
            extra.insert("parallel_tool_calls".into(), serde_json::json!(true));
            builder = builder.additional_params(serde_json::Value::Object(extra));
        }

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
                .tool(super::tools::ScheduleAdd {
                    scheduler: scheduler.clone(),
                    is_owner: params.is_owner,
                    discord_channel_id: params.discord_channel_id,
                })
                .tool(super::tools::ScheduleDelete {
                    scheduler: scheduler.clone(),
                    is_owner: params.is_owner,
                })
                .tool(super::tools::ScheduleList {
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
                    let final_text = res.response().to_string();
                    if response_text.is_empty() {
                        let _ = tx.send(StreamEvent::TextDelta(final_text.clone())).await;
                        response_text = final_text;
                    } else if let Some(remaining) = final_text.strip_prefix(&response_text) {
                        if !remaining.is_empty() {
                            let _ = tx.send(StreamEvent::TextDelta(remaining.to_string())).await;
                        }
                        response_text = final_text;
                    } else {
                        let _ = tx.send(StreamEvent::TextDelta(final_text.clone())).await;
                        response_text = final_text;
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

#[async_trait]
impl<C> Agent for RigAgent<C>
where
    C: CompletionClient + Send + Sync,
    C::CompletionModel: 'static,
{
    async fn set_scheduler(&self, scheduler: Arc<Scheduler>) {
        *self.scheduler.write().await = Some(scheduler);
    }

    async fn process_streaming(
        &self,
        user_input: &str,
        is_owner: bool,
        discord_channel_id: Option<u64>,
        user_info: Option<&UserInfo>,
        attachments: &[AttachmentInfo],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<AgentResponse> {
        let context = self.memory.get_context(user_input).await?;
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

        let author = user_info.map(|u| u.name.as_str()).unwrap_or("User");
        self.memory.add_turn(author, user_input, &response).await?;
        let files = pending_files.read().await.clone();

        Ok(AgentResponse {
            text: response,
            files,
        })
    }
}

pub async fn create_agent(config: Config, memory: Arc<MemoryManager>) -> Result<Arc<dyn Agent>> {
    match config.api_provider.as_str() {
        "openai" => {
            let client: openai::CompletionsClient = openai::CompletionsClient::builder()
                .api_key(&config.api_key)
                .base_url(&config.api_url)
                .build()?;
            let agent = RigAgent::new(config, memory, client).await?;
            Ok(agent as Arc<dyn Agent>)
        }
        "gemini" => {
            let client = gemini::Client::new(&config.api_key)?;
            let agent = RigAgent::new(config, memory, client).await?;
            Ok(agent as Arc<dyn Agent>)
        }
        _ => {
            let client: anthropic::Client = anthropic::Client::builder()
                .api_key(&config.api_key)
                .base_url(&config.api_url)
                .build()?;
            let agent = RigAgent::new(config, memory, client).await?;
            Ok(agent as Arc<dyn Agent>)
        }
    }
}
