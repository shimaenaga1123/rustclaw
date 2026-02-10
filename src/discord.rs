use crate::{
    agent::{Agent, AttachmentInfo, StreamEvent, UserInfo},
    config::Config,
    scheduler::Scheduler,
    utils,
};
use anyhow::Result;
use serenity::builder::EditMessage;
use serenity::{
    all::CreateAttachment,
    async_trait,
    builder::CreateMessage,
    model::{channel::Message, gateway::Ready, id::UserId},
    prelude::*,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

const DISCORD_MAX_LEN: usize = 2000;
const MAX_ATTACHMENT_SIZE: u32 = 25 * 1024 * 1024; // 25MB

pub struct Bot {
    config: Config,
    agent: Arc<dyn Agent>,
    scheduler: Arc<Scheduler>,
}

struct Handler {
    agent: Arc<dyn Agent>,
    config: Config,
    bot_id: Arc<RwLock<Option<UserId>>>,
    owner_id: UserId,
    scheduler: Arc<Scheduler>,
    http_client: reqwest::Client,
}

impl Handler {
    async fn build_user_info(&self, ctx: &Context, msg: &Message) -> UserInfo {
        let author = &msg.author;

        let mut user_info = UserInfo {
            name: author.name.clone(),
            global_name: author.global_name.clone(),
            id: author.id.get(),
            avatar_url: author.avatar_url(),
            ..Default::default()
        };

        if let Some(guild_id) = msg.guild_id
            && let Some(ref member) = msg.member
        {
            user_info.nickname = member.nick.clone();

            if let Some(guild) = ctx.cache.guild(guild_id) {
                let role_names: Vec<String> = member
                    .roles
                    .iter()
                    .filter_map(|role_id| guild.roles.get(role_id).map(|r| r.name.clone()))
                    .collect();
                user_info.roles = role_names;
            }
        }

        user_info
    }

    async fn download_attachments(&self, msg: &Message) -> Vec<AttachmentInfo> {
        if msg.attachments.is_empty() {
            return Vec::new();
        }

        let upload_dir = self.upload_dir();
        if let Err(e) = tokio::fs::create_dir_all(&upload_dir).await {
            error!("Failed to create upload directory: {}", e);
            return Vec::new();
        }

        let mut result = Vec::new();

        for attachment in &msg.attachments {
            if attachment.size > MAX_ATTACHMENT_SIZE {
                warn!(
                    "Skipping attachment '{}': too large ({} bytes)",
                    attachment.filename, attachment.size
                );
                continue;
            }

            let safe_filename = sanitize_filename(&attachment.filename);
            if safe_filename.is_empty() {
                warn!("Skipping attachment with invalid filename");
                continue;
            }

            let host_path = upload_dir.join(&safe_filename);
            let final_filename = if host_path.exists() {
                let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
                let stem = safe_filename
                    .rfind('.')
                    .map(|i| &safe_filename[..i])
                    .unwrap_or(&safe_filename);
                let ext = safe_filename
                    .rfind('.')
                    .map(|i| &safe_filename[i..])
                    .unwrap_or("");
                format!("{}_{}{}", stem, ts, ext)
            } else {
                safe_filename
            };

            let host_path = upload_dir.join(&final_filename);

            match self.http_client.get(&attachment.url).send().await {
                Ok(response) if response.status().is_success() => match response.bytes().await {
                    Ok(bytes) => {
                        if let Err(e) = tokio::fs::write(&host_path, &bytes).await {
                            error!("Failed to write attachment '{}': {}", final_filename, e);
                            continue;
                        }

                        info!(
                            "Downloaded attachment '{}' ({} bytes)",
                            final_filename,
                            bytes.len()
                        );

                        result.push(AttachmentInfo {
                            filename: final_filename.clone(),
                            container_path: format!("/workspace/upload/{}", final_filename),
                            size: attachment.size,
                            content_type: attachment.content_type.clone(),
                        });
                    }
                    Err(e) => {
                        error!(
                            "Failed to read attachment bytes '{}': {}",
                            final_filename, e
                        );
                    }
                },
                Ok(response) => {
                    error!(
                        "Failed to download attachment '{}': HTTP {}",
                        final_filename,
                        response.status()
                    );
                }
                Err(e) => {
                    error!("Failed to download attachment '{}': {}", final_filename, e);
                }
            }
        }

        result
    }

    fn upload_dir(&self) -> PathBuf {
        self.config.data_dir.join("workspace").join("upload")
    }
}

fn sanitize_filename(name: &str) -> String {
    name.replace(['/', '\\', '\0', ':', '*', '?', '"', '<', '>', '|'], "_")
        .trim()
        .chars()
        .take(200)
        .collect()
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

        if content.is_empty() && msg.attachments.is_empty() {
            return;
        }

        if let Err(e) = msg.react(&ctx, '👀').await {
            error!("Failed to add reaction: {}", e);
        }

        let typing = msg.channel_id.start_typing(&ctx.http);

        let user_info = self.build_user_info(&ctx, &msg).await;

        let attachments = self.download_attachments(&msg).await;

        let is_owner = msg.author.id == self.owner_id;

        let input = if content.is_empty() {
            "I've attached files. Please check them.".to_string()
        } else {
            content
        };

        let (tx, mut rx) = mpsc::channel::<StreamEvent>(128);

        let agent = self.agent.clone();
        let channel_id_val = msg.channel_id.get();
        let attachments_owned = attachments;

        let handle = tokio::spawn(async move {
            agent
                .process_streaming(
                    &input,
                    is_owner,
                    Some(channel_id_val),
                    Some(&user_info),
                    &attachments_owned,
                    tx,
                )
                .await
        });

        typing.stop();

        let mut reply_msg = match msg.reply(&ctx, "…").await {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to send initial reply: {}", e);
                return;
            }
        };

        let mut accumulated = String::new();
        let mut last_edit = Instant::now();
        let edit_interval = Duration::from_millis(800);
        let mut extra_messages: Vec<Message> = Vec::new();

        loop {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(StreamEvent::TextDelta(text))) => {
                    accumulated.push_str(&text);

                    if accumulated.len() > 1900 {
                        let split_at = accumulated[..1900]
                            .rfind('\n')
                            .or_else(|| accumulated[..1900].rfind(' '))
                            .unwrap_or(1900);

                        let current_chunk: String = accumulated[..split_at].to_string();
                        accumulated = accumulated[split_at..].to_string();

                        if extra_messages.is_empty() {
                            let _ = reply_msg
                                .edit(&ctx, EditMessage::new().content(&current_chunk))
                                .await;
                        } else {
                            let last = extra_messages.last_mut().unwrap();
                            let _ = last
                                .edit(&ctx, EditMessage::new().content(&current_chunk))
                                .await;
                        }

                        match msg.channel_id.say(&ctx.http, "…").await {
                            Ok(m) => extra_messages.push(m),
                            Err(e) => error!("Failed to send continuation message: {}", e),
                        }
                    }

                    if last_edit.elapsed() >= edit_interval && !accumulated.is_empty() {
                        let target = if let Some(last) = extra_messages.last_mut() {
                            last
                        } else {
                            &mut reply_msg
                        };
                        let _ = target
                            .edit(&ctx, EditMessage::new().content(&accumulated))
                            .await;
                        last_edit = Instant::now();
                    }
                }
                Ok(Some(StreamEvent::Done)) | Ok(None) => break,
                Ok(Some(StreamEvent::Error(e))) => {
                    error!("Stream error: {}", e);
                    break;
                }
                Err(_) => {
                    if last_edit.elapsed() >= edit_interval && !accumulated.is_empty() {
                        let target = if let Some(last) = extra_messages.last_mut() {
                            last
                        } else {
                            &mut reply_msg
                        };
                        let _ = target
                            .edit(&ctx, EditMessage::new().content(&accumulated))
                            .await;
                        last_edit = Instant::now();
                    }
                }
            }
        }

        if !accumulated.is_empty() {
            let target = if let Some(last) = extra_messages.last_mut() {
                last
            } else {
                &mut reply_msg
            };
            let _ = target
                .edit(&ctx, EditMessage::new().content(&accumulated))
                .await;
        }

        match handle.await {
            Ok(Ok(response)) => {
                if accumulated.is_empty() && !response.text.is_empty() {
                    let chunks = utils::split_message(&response.text, DISCORD_MAX_LEN);
                    let _ = reply_msg
                        .edit(&ctx, EditMessage::new().content(&chunks[0]))
                        .await;
                    for chunk in &chunks[1..] {
                        let _ = msg.channel_id.say(&ctx.http, chunk).await;
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
            Ok(Err(e)) => {
                error!("Agent error: {}", e);
                let _ = reply_msg
                    .edit(
                        &ctx,
                        EditMessage::new().content("처리 중 오류가 발생했습니다."),
                    )
                    .await;
            }
            Err(e) => {
                error!("Task join error: {}", e);
                let _ = reply_msg
                    .edit(
                        &ctx,
                        EditMessage::new().content("처리 중 오류가 발생했습니다."),
                    )
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
            | GatewayIntents::MESSAGE_CONTENT;

        let handler = Handler {
            agent: self.agent,
            config: self.config.clone(),
            bot_id: Arc::new(RwLock::new(None)),
            owner_id: UserId::new(self.config.owner_id),
            scheduler: self.scheduler,
            http_client: reqwest::Client::new(),
        };

        let mut client = Client::builder(&self.config.discord_token, intents)
            .event_handler(handler)
            .await?;

        client.start().await?;

        Ok(())
    }
}
