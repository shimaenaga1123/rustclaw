use super::{AttachmentInfo, PendingFile, UserInfo, preamble::build_preamble};
use crate::config::Config;
use crate::memory::MemoryManager;
use crate::scheduler::Scheduler;
use crate::tools;
use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;
use rig::{
    agent::MultiTurnStreamItem,
    client::CompletionClient,
    completion::{CompletionModel, GetTokenUsage},
    streaming::{StreamedAssistantContent, StreamingPrompt},
};
use rustypipe::client::RustyPipe;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    Done,
    Error(String),
}

struct StreamParams {
    model: String,
    preamble: String,
    prompt: String,
    disable_reasoning: bool,
    is_owner: bool,
    discord_channel_id: Option<u64>,
    config: Arc<Config>,
    memory: Arc<MemoryManager>,
    scheduler: Option<Arc<Scheduler>>,
    pending_files: Arc<RwLock<Vec<PendingFile>>>,
    tx: mpsc::Sender<StreamEvent>,
}

pub struct AgentResponse {
    pub text: String,
    pub files: Vec<PendingFile>,
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
    http_client: reqwest::Client,
    rp: Arc<RustyPipe>,
    client: C,
}

impl<C: CompletionClient> RigAgent<C> {
    pub async fn new(config: Config, memory: Arc<MemoryManager>, client: C) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            config,
            memory,
            scheduler: RwLock::new(None),
            http_client: reqwest::Client::new(),
            rp: Arc::new(RustyPipe::new()),
            client,
        }))
    }

    async fn stream_prompt(&self, params: StreamParams) -> Result<String>
    where
        <C as CompletionClient>::CompletionModel: 'static,
    {
        let vector_db = params.memory.vector_db().clone();

        let mut builder = self
            .client
            .agent(params.model)
            .preamble(params.preamble.as_str())
            .tool(tools::RunCommand {
                config: params.config.clone(),
            })
            .tool(tools::ImportantList {
                vectordb: vector_db.clone(),
            })
            .tool(tools::SendFile {
                pending_files: params.pending_files.clone(),
                config: params.config.clone(),
            })
            .tool(tools::TypstRender {
                pending_files: params.pending_files.clone(),
                config: params.config.clone(),
            })
            .tool(tools::SearchMemory {
                vectordb: vector_db.clone(),
            })
            .tool(tools::Weather {
                client: self.http_client.clone(),
            })
            .tool(tools::SearchYouTube {
                rp: self.rp.clone(),
            })
            .tool(tools::GetTranscript {
                rp: self.rp.clone(),
                client: self.http_client.clone(),
            });

        {
            let mut extra = serde_json::Map::new();
            if params.disable_reasoning {
                extra.insert("thinking".into(), serde_json::json!({"type": "disabled"}));
            }
            extra.insert("parallel_tool_calls".into(), serde_json::json!(false));
            builder = builder.additional_params(serde_json::Value::Object(extra));
        }

        if params.is_owner {
            builder = builder
                .tool(tools::ImportantAdd {
                    vectordb: vector_db.clone(),
                    is_owner: true,
                })
                .tool(tools::ImportantDelete {
                    vectordb: vector_db.clone(),
                    is_owner: true,
                })
                .tool(tools::ResetContainer {
                    config: params.config.clone(),
                });
        }

        if params.config.search.api_key.is_some() {
            builder = builder.tool(tools::WebSearch {
                config: params.config.clone(),
                client: self.http_client.clone(),
            });

            if params.config.search.provider.as_deref().unwrap_or("") == "serper" {
                builder = builder.tool(tools::WebNews {
                    config: params.config.clone(),
                    client: self.http_client.clone(),
                });
            }
        }

        if let Some(ref scheduler) = params.scheduler {
            builder = builder
                .tool(tools::ScheduleAdd {
                    scheduler: scheduler.clone(),
                    is_owner: params.is_owner,
                    discord_channel_id: params.discord_channel_id,
                })
                .tool(tools::ScheduleList {
                    scheduler: scheduler.clone(),
                });
            if params.is_owner {
                builder = builder.tool(tools::ScheduleDelete {
                    scheduler: scheduler.clone(),
                    is_owner: true,
                });
            }
        }

        Self::run_stream(
            builder.default_max_turns(50).build(),
            params.prompt.as_str(),
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
        <A as StreamingPrompt<M, R>>::Hook: 'static,
    {
        let mut stream = agent.stream_prompt(prompt).await;
        let mut response_text = String::new();

        while let Some(item) = stream.next().await {
            match item {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    text,
                ))) => {
                    let delta = text.text;
                    response_text.push_str(&delta);
                    let _ = tx.send(StreamEvent::TextDelta(delta)).await;
                }
                Ok(MultiTurnStreamItem::FinalResponse(res)) => {
                    let final_text = res.response().to_string();
                    if response_text.is_empty() {
                        let _ = tx.send(StreamEvent::TextDelta(final_text.clone())).await;
                    } else if let Some(remaining) = final_text.strip_prefix(&response_text)
                        && !remaining.is_empty()
                    {
                        let _ = tx.send(StreamEvent::TextDelta(remaining.to_string())).await;
                    }
                    response_text = final_text;
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
        full_prompt.reserve(
            context.len() + user_section.len() + attachment_section.len() + user_input.len() + 16,
        );
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
        full_prompt.push_str("User: ");
        full_prompt.push_str(user_input);

        let scheduler_ref = self.scheduler.read().await.clone();
        let preamble = build_preamble(
            is_owner,
            scheduler_ref.is_some(),
            self.config.search.api_key.is_some(),
            self.config.search.api_key.is_some()
                && self.config.search.provider.as_deref().unwrap_or("") == "serper",
        );
        let pending_files = Arc::new(RwLock::new(Vec::new()));

        let response = self
            .stream_prompt(StreamParams {
                model: self.config.api.model.clone(),
                preamble,
                prompt: full_prompt,
                disable_reasoning: self.config.model.disable_reasoning,
                is_owner,
                discord_channel_id,
                config: Arc::new(self.config.clone()),
                memory: self.memory.clone(),
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
