use crate::{agent::RigAgent, config::Config, memory::MemoryManager, scheduler::Scheduler};
use anyhow::Result;
use serenity::{
    all::CreateAttachment,
    async_trait,
    builder::CreateMessage,
    model::{channel::Message, gateway::Ready, id::UserId},
    prelude::*,
};
use std::sync::Arc;
use tracing::{error, info};

const DISCORD_MAX_LEN: usize = 2000;

pub struct Bot {
    config: Config,
    agent: Arc<RigAgent>,
    memory: Arc<MemoryManager>,
    scheduler: Arc<Scheduler>,
}

struct Handler {
    agent: Arc<RigAgent>,
    memory: Arc<MemoryManager>,
    bot_id: Arc<RwLock<Option<UserId>>>,
    owner_id: UserId,
    scheduler: Arc<Scheduler>,
}

fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        let split_at = remaining[..max_len]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or_else(|| {
                remaining[..max_len]
                    .rfind(' ')
                    .map(|i| i + 1)
                    .unwrap_or(max_len)
            });

        let chunk = &remaining[..split_at];
        if !chunk.trim().is_empty() {
            chunks.push(chunk.to_string());
        }
        remaining = &remaining[split_at..];
    }

    if chunks.is_empty() {
        chunks.push(text.chars().take(max_len).collect());
    }

    chunks
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        let bot_id = self.bot_id.read().await;
        let Some(bot_id) = *bot_id else {
            return;
        };

        if !msg.mentions.iter().any(|u| u.id == bot_id) {
            return;
        }

        let content = msg
            .content
            .replace(&format!("<@{}>", bot_id), "")
            .trim()
            .to_string();

        if content.is_empty() {
            return;
        }

        if let Err(e) = msg.react(&ctx, '👀').await {
            error!("Failed to add reaction: {}", e);
        }

        let typing = msg.channel_id.start_typing(&ctx.http);

        if let Err(e) = self.memory.add_message(&msg.author.name, &content).await {
            error!("Failed to add message to memory: {}", e);
            typing.stop();
            return;
        }

        let is_owner = msg.author.id == self.owner_id;

        match self
            .agent
            .process(&content, is_owner, Some(msg.channel_id.get()))
            .await
        {
            Ok(response) => {
                typing.stop();

                let chunks = split_message(&response.text, DISCORD_MAX_LEN);
                for (i, chunk) in chunks.iter().enumerate() {
                    let result = if i == 0 {
                        msg.reply(&ctx, chunk).await
                    } else {
                        msg.channel_id.say(&ctx.http, chunk).await
                    };
                    if let Err(e) = result {
                        error!("Failed to send message chunk {}: {}", i, e);
                    }
                }

                for file in &response.files {
                    match CreateAttachment::path(&file.path).await {
                        Ok(attachment) => {
                            let builder = CreateMessage::new().add_file(attachment);
                            if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
                                error!("Failed to send file '{}': {}", file.filename, e);
                            }
                        }
                        Err(e) => {
                            error!("Failed to create attachment for '{}': {}", file.filename, e);
                        }
                    }

                    if file.path.to_string_lossy().contains("/tmp/")
                        && let Err(e) = tokio::fs::remove_file(&file.path).await
                    {
                        error!("Failed to clean up temp file: {}", e);
                    }
                }
            }
            Err(e) => {
                typing.stop();
                error!("Agent error: {}", e);
                let _ = msg
                    .reply(&ctx, "An error occurred during processing.")
                    .await;
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Bot connected as {}", ready.user.name);
        *self.bot_id.write().await = Some(ready.user.id);
        self.scheduler.set_discord_http(ctx.http).await;
    }
}

impl Bot {
    pub async fn new(
        config: Config,
        agent: Arc<RigAgent>,
        memory: Arc<MemoryManager>,
        scheduler: Arc<Scheduler>,
    ) -> Result<Self> {
        Ok(Self {
            config,
            agent,
            memory,
            scheduler,
        })
    }

    pub async fn start(self) -> Result<()> {
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;

        let handler = Handler {
            agent: self.agent,
            memory: self.memory,
            bot_id: Arc::new(RwLock::new(None)),
            owner_id: UserId::new(self.config.owner_id),
            scheduler: self.scheduler,
        };

        let mut client = Client::builder(&self.config.discord_token, intents)
            .event_handler(handler)
            .await?;

        client.start().await?;

        Ok(())
    }
}
