use crate::{agent::RigAgent, config::Config, memory::MemoryManager, scheduler::Scheduler};
use anyhow::Result;
use serenity::{
    async_trait,
    model::{channel::Message, gateway::Ready, id::UserId},
    prelude::*,
};
use std::sync::Arc;
use tracing::{error, info};

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
                if let Err(e) = msg.reply(&ctx, response).await {
                    error!("Failed to send message: {}", e);
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
