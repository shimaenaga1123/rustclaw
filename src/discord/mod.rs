use anyhow::Result;
use serenity::{model::id::UserId, prelude::*};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;

mod handler;
mod relay;
mod util;

use crate::{agent::Agent, config::Config, scheduler::Scheduler};
pub(crate) use handler::send_agent_response;
use handler::*;
use relay::*;
use util::*;

pub const DISCORD_MAX_LEN: usize = 2000;
pub const MAX_ATTACHMENT_SIZE: u32 = 25 * 1024 * 1024; // 25MB
pub const CANCEL_EMOJI: char = '❌';

pub const EDIT_INTERVAL: Duration = Duration::from_millis(800);
pub const STREAM_POLL_TIMEOUT: Duration = Duration::from_millis(200);
pub const NOTIFY_DELETE_DELAY: Duration = Duration::from_secs(2);

pub struct Bot {
    config: Config,
    agent: Arc<dyn Agent>,
    scheduler: Arc<Scheduler>,
}

impl Bot {
    pub async fn new(
        config: Config,
        agent: Arc<dyn Agent>,
        scheduler: Arc<Scheduler>,
    ) -> Result<Self> {
        Ok(Self {
            config,
            agent,
            scheduler,
        })
    }

    pub async fn start(self) -> Result<()> {
        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILD_MESSAGE_REACTIONS
            | GatewayIntents::DIRECT_MESSAGE_REACTIONS;

        let handler = Handler {
            agent: self.agent,
            config: self.config.clone(),
            bot_id: Arc::new(RwLock::new(None)),
            owner_id: UserId::new(self.config.owner_id),
            scheduler: self.scheduler,
            http_client: reqwest::Client::new(),
            active_streams: Arc::new(Mutex::new(HashMap::new())),
        };

        let mut client = Client::builder(&self.config.discord_token, intents)
            .event_handler(handler)
            .await?;

        client.start().await?;

        Ok(())
    }
}
