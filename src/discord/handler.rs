use super::*;
use crate::{
    agent::{Agent, AttachmentInfo, StreamEvent, UserInfo},
    config::Config,
    scheduler::Scheduler,
};
use serenity::{
    all::{CreateAttachment, Http},
    async_trait,
    builder::{
        CreateCommand, CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage,
        EditMessage,
    },
    model::{
        application::Interaction,
        channel::{Message, Reaction},
        gateway::Ready,
        id::{ChannelId, MessageId, UserId},
    },
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex, mpsc};
use tracing::{error, info, warn};

pub(super) struct StreamControl {
    pub abort_handle: tokio::task::AbortHandle,
    pub cancelled: Arc<AtomicBool>,
    pub requester_id: UserId,
    pub channel_id: ChannelId,
}

pub(super) struct Handler {
    pub agent: Arc<dyn Agent>,
    pub config: Config,
    pub bot_id: Arc<RwLock<Option<UserId>>>,
    pub owner_id: UserId,
    pub scheduler: Arc<Scheduler>,
    pub http_client: reqwest::Client,
    pub active_streams: Arc<Mutex<HashMap<MessageId, StreamControl>>>,
}

async fn send_pending_file(
    http: &Http,
    channel_id: ChannelId,
    file: &crate::agent::PendingFile,
) -> Result<(), String> {
    let attachment = CreateAttachment::path(&file.path)
        .await
        .map_err(|e| e.to_string())?;
    channel_id
        .send_message(http, CreateMessage::new().add_file(attachment))
        .await
        .map_err(|e| e.to_string())?;

    if file.path.to_string_lossy().contains("/tmp/")
        && let Err(e) = tokio::fs::remove_file(&file.path).await
    {
        error!("Failed to clean up temp file: {}", e);
    }

    Ok(())
}

pub async fn send_agent_response(
    http: &Http,
    channel_id: u64,
    response: &crate::agent::AgentResponse,
) {
    let channel = ChannelId::new(channel_id);

    for chunk in split_message(&response.text, DISCORD_MAX_LEN) {
        if let Err(e) = channel.say(http, &chunk).await {
            error!(
                "Failed to send response chunk to Discord channel {}: {}",
                channel_id, e
            );
        }
    }

    for file in &response.files {
        if let Err(e) = send_pending_file(http, channel, file).await {
            error!(
                "Failed to send file '{}' to Discord channel {}: {}",
                file.filename, channel_id, e
            );
        }
    }
}

impl Handler {
    pub(super) async fn build_user_info(&self, ctx: &Context, msg: &Message) -> UserInfo {
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
                user_info.roles = member
                    .roles
                    .iter()
                    .filter_map(|role_id| guild.roles.get(role_id).map(|r| r.name.clone()))
                    .collect();
            }
        }

        user_info
    }

    pub(super) async fn download_attachments(&self, msg: &Message) -> Vec<AttachmentInfo> {
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

            let final_filename = deduplicate_filename(&upload_dir, &safe_filename);
            let host_path = upload_dir.join(&final_filename);

            match self
                .download_single_attachment(&attachment.url, &host_path)
                .await
            {
                Ok(size) => {
                    info!(
                        "Downloaded attachment '{}' ({} bytes)",
                        final_filename, size
                    );
                    result.push(AttachmentInfo {
                        filename: final_filename.clone(),
                        container_path: format!("/workspace/upload/{}", final_filename),
                        size: attachment.size,
                        content_type: attachment.content_type.clone(),
                    });
                }
                Err(e) => {
                    error!("Failed to download attachment '{}': {}", final_filename, e);
                }
            }
        }

        result
    }

    async fn download_single_attachment(&self, url: &str, dest: &PathBuf) -> Result<usize, String> {
        let response = self
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }

        let bytes = response.bytes().await.map_err(|e| e.to_string())?;
        let size = bytes.len();
        tokio::fs::write(dest, &bytes)
            .await
            .map_err(|e| e.to_string())?;

        Ok(size)
    }

    fn upload_dir(&self) -> PathBuf {
        self.config
            .storage
            .data_dir
            .join("workspace")
            .join("upload")
    }

    fn extract_content(msg: &Message, bot_id: UserId) -> String {
        msg.content
            .replace(&format!("<@{}>", bot_id), "")
            .trim()
            .to_string()
    }

    async fn run_stream_loop(
        &self,
        ctx: &Context,
        relay: &mut StreamRelay,
        rx: &mut mpsc::Receiver<StreamEvent>,
        origin_channel: ChannelId,
    ) {
        loop {
            match tokio::time::timeout(STREAM_POLL_TIMEOUT, rx.recv()).await {
                Ok(Some(StreamEvent::TextDelta(text))) => {
                    relay
                        .push_delta(ctx, &text, origin_channel, &self.active_streams)
                        .await;
                }
                Ok(Some(StreamEvent::Done)) | Ok(None) => break,
                Ok(Some(StreamEvent::Error(e))) => {
                    error!("Stream error: {}", e);
                    break;
                }
                Err(_) => {
                    if relay.last_edit.elapsed() >= EDIT_INTERVAL && !relay.accumulated.is_empty() {
                        relay.flush_edit(ctx).await;
                    }
                }
            }
        }
    }

    async fn send_response_files_and_notify(
        &self,
        ctx: &Context,
        msg: &Message,
        relay: &mut StreamRelay,
        response: crate::agent::AgentResponse,
    ) {
        if relay.accumulated.is_empty() && !response.text.is_empty() {
            let chunks = split_message(&response.text, DISCORD_MAX_LEN);
            let _ = relay
                .reply_msg
                .edit(ctx, EditMessage::new().content(&chunks[0]))
                .await;
            for chunk in &chunks[1..] {
                let _ = msg.channel_id.say(&ctx.http, chunk).await;
            }
        }

        for file in &response.files {
            if let Err(e) = send_pending_file(&ctx.http, msg.channel_id, file).await {
                error!("Failed to send file '{}': {}", file.filename, e);
            }
        }

        self.send_ephemeral_mention(ctx, msg.channel_id, msg.author.id)
            .await;
    }

    async fn send_ephemeral_mention(&self, ctx: &Context, channel_id: ChannelId, user_id: UserId) {
        if let Ok(notify_msg) = channel_id.say(&ctx.http, format!("<@{}>", user_id)).await {
            let http = ctx.http.clone();
            tokio::spawn(async move {
                tokio::time::sleep(NOTIFY_DELETE_DELAY).await;
                let _ = channel_id.delete_message(&http, notify_msg.id).await;
            });
        }
    }

    async fn unregister_stream(&self, ctx: &Context, relay: &StreamRelay, channel_id: ChannelId) {
        self.active_streams
            .lock()
            .await
            .remove(&relay.cancel_msg_id);
        let _ = ctx
            .http
            .delete_message_reaction_emoji(channel_id, relay.cancel_msg_id, &cancel_emoji())
            .await;
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        let bot_id = match *self.bot_id.read().await {
            Some(id) => id,
            None => return,
        };

        if !msg.mentions.iter().any(|u| u.id == bot_id) {
            return;
        }

        let content = Self::extract_content(&msg, bot_id);
        if content.is_empty() && msg.attachments.is_empty() {
            return;
        }

        let _ = msg.react(&ctx, '👀').await;
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
        let handle = tokio::spawn(async move {
            agent
                .process_streaming(
                    &input,
                    is_owner,
                    Some(channel_id_val),
                    Some(&user_info),
                    &attachments,
                    tx,
                )
                .await
        });

        typing.stop();

        let reply_msg = match msg.reply(&ctx, "…").await {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to send initial reply: {}", e);
                handle.abort();
                return;
            }
        };

        let _ = reply_msg.react(&ctx, CANCEL_EMOJI).await;

        let cancelled = Arc::new(AtomicBool::new(false));
        let mut relay = StreamRelay::new(reply_msg);

        self.active_streams.lock().await.insert(
            relay.cancel_msg_id,
            StreamControl {
                abort_handle: handle.abort_handle(),
                cancelled: cancelled.clone(),
                requester_id: msg.author.id,
                channel_id: msg.channel_id,
            },
        );

        self.run_stream_loop(&ctx, &mut relay, &mut rx, msg.channel_id)
            .await;

        self.unregister_stream(&ctx, &relay, msg.channel_id).await;

        if cancelled.load(Ordering::Acquire) {
            relay.finalize(&ctx, true).await;
            return;
        }

        relay.finalize(&ctx, false).await;

        match handle.await {
            Ok(Ok(response)) => {
                self.send_response_files_and_notify(&ctx, &msg, &mut relay, response)
                    .await;
            }
            Ok(Err(e)) => {
                error!("Agent error: {}", e);
                let _ = relay
                    .reply_msg
                    .edit(
                        &ctx,
                        EditMessage::new().content("An error occurred while processing."),
                    )
                    .await;
            }
            Err(e) if e.is_cancelled() => {}
            Err(e) => {
                error!("Task join error: {}", e);
                let _ = relay
                    .reply_msg
                    .edit(
                        &ctx,
                        EditMessage::new().content("An error occurred while processing."),
                    )
                    .await;
            }
        }
    }

    async fn reaction_add(&self, ctx: Context, reaction: Reaction) {
        if reaction.emoji != cancel_emoji() {
            return;
        }

        let bot_id = *self.bot_id.read().await;
        if reaction.user_id == bot_id {
            return;
        }

        let Some(user_id) = reaction.user_id else {
            return;
        };

        let is_admin = user_id == self.owner_id;

        let mut streams = self.active_streams.lock().await;
        let Some(ctrl) = streams.get(&reaction.message_id) else {
            return;
        };

        if !is_admin && user_id != ctrl.requester_id {
            let emoji = reaction.emoji.clone();
            let _ = ctx
                .http
                .delete_reaction(reaction.channel_id, reaction.message_id, user_id, &emoji)
                .await;
            return;
        }

        let Some(ctrl) = streams.remove(&reaction.message_id) else {
            return;
        };
        drop(streams);

        ctrl.cancelled.store(true, Ordering::Release);
        ctrl.abort_handle.abort();
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Bot connected as {}", ready.user.name);
        *self.bot_id.write().await = Some(ready.user.id);
        self.scheduler.set_discord_http(ctx.http.clone()).await;

        if let Err(e) = serenity::model::application::Command::create_global_command(
            &ctx.http,
            CreateCommand::new("cancelall")
                .description("(Admin only) Cancel all active AI response streams"),
        )
        .await
        {
            error!("Failed to register /cancelall slash command: {}", e);
        } else {
            info!("Registered /cancelall slash command");
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(cmd) = interaction else {
            return;
        };

        if cmd.data.name != "cancelall" {
            return;
        }

        if cmd.user.id != self.owner_id {
            let response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("You don't have permission to use this command.")
                    .ephemeral(true),
            );
            if let Err(e) = cmd.create_response(&ctx.http, response).await {
                error!("Failed to respond to /cancelall: {}", e);
            }
            return;
        }

        let channel_id = cmd.channel_id;
        let mut streams = self.active_streams.lock().await;
        let keys: Vec<MessageId> = streams
            .iter()
            .filter(|(_, ctrl)| ctrl.channel_id == channel_id)
            .map(|(id, _)| *id)
            .collect();

        let reply = if keys.is_empty() {
            "No active streams to cancel.".to_string()
        } else {
            let count = keys.len();
            for key in keys {
                let Some(ctrl) = streams.remove(&key) else {
                    return;
                };
                ctrl.cancelled.store(true, Ordering::Release);
                ctrl.abort_handle.abort();
            }
            format!("Cancelled {} stream(s).", count)
        };
        drop(streams);

        let response = CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content(reply)
                .ephemeral(true),
        );
        if let Err(e) = cmd.create_response(&ctx.http, response).await {
            error!("Failed to respond to /cancelall: {}", e);
        }
    }
}
