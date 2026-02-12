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
           Use this when output is too long for a message, or when generating files.\n\
         - **typst_render**: Renders Typst markup to a PNG image and sends it as a Discord attachment. \
           Use this for tables, math equations ($x^2 + y^2 = z^2$), or any formatted content \
           that Discord markdown cannot display. Write valid Typst markup.\n\
         - **search_memory**: Searches past conversations semantically. Use when the user asks \
           about previous discussions or when you need to recall something from past context \
           beyond the recent/related turns already in the prompt.\n\
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
         You have access to a vector-based memory system powered by usearch.\n\
         - **Important Facts**: Key facts appear under '# Important Facts'. Use important_add to save new ones (owner only).\n\
         - **Recent Conversations**: The last 5 conversation turns are included for continuity.\n\
         - **Related Past Conversations**: Semantically similar past conversations are automatically retrieved.\n\
         Use important_add proactively when the owner shares preferences, important dates, or key decisions.\n\n",
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
