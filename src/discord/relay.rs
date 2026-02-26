use super::*;
use serenity::{
    builder::EditMessage,
    model::{
        channel::Message,
        id::{ChannelId, MessageId},
    },
};
use std::{collections::HashMap, time::Instant};
use tokio::sync::Mutex;
use tracing::error;

pub(super) struct StreamRelay {
    pub reply_msg: Message,
    overflow: Vec<Message>,
    pub accumulated: String,
    pub last_edit: Instant,
    pub cancel_msg_id: MessageId,
}

impl StreamRelay {
    pub fn new(reply_msg: Message) -> Self {
        let cancel_msg_id = reply_msg.id;
        Self {
            reply_msg,
            overflow: Vec::new(),
            accumulated: String::new(),
            last_edit: Instant::now(),
            cancel_msg_id,
        }
    }

    fn current_target(&mut self) -> &mut Message {
        self.overflow.last_mut().unwrap_or(&mut self.reply_msg)
    }

    pub async fn flush_edit(&mut self, ctx: &Context) {
        if self.accumulated.is_empty() {
            return;
        }
        let content = self.accumulated.clone();
        let _ = self
            .current_target()
            .edit(ctx, EditMessage::new().content(&content))
            .await;
        self.last_edit = Instant::now();
    }

    pub async fn push_delta(
        &mut self,
        ctx: &Context,
        text: &str,
        origin_channel: ChannelId,
        active_streams: &Mutex<HashMap<MessageId, StreamControl>>,
    ) {
        self.accumulated.push_str(text);

        if self.accumulated.chars().count() > DISCORD_MAX_LEN {
            let (finished_chunk, rest) = split_streaming(&self.accumulated, DISCORD_MAX_LEN);
            self.accumulated = rest;

            let _ = self
                .current_target()
                .edit(ctx, EditMessage::new().content(&finished_chunk))
                .await;

            if let Ok(new_msg) = origin_channel.say(&ctx.http, "…").await {
                self.move_cancel_emoji(ctx, origin_channel, &new_msg, active_streams)
                    .await;
                self.overflow.push(new_msg);
            }
        }

        if self.last_edit.elapsed() >= EDIT_INTERVAL && !self.accumulated.is_empty() {
            self.flush_edit(ctx).await;
        }
    }

    async fn move_cancel_emoji(
        &mut self,
        ctx: &Context,
        channel_id: ChannelId,
        new_msg: &Message,
        active_streams: &Mutex<HashMap<MessageId, StreamControl>>,
    ) {
        let _ = ctx
            .http
            .delete_message_reaction_emoji(channel_id, self.cancel_msg_id, &cancel_emoji())
            .await;

        if let Err(e) = new_msg.react(ctx, CANCEL_EMOJI).await {
            error!("Failed to move cancel reaction: {}", e);
        }

        let mut streams = active_streams.lock().await;
        if let Some(ctrl) = streams.remove(&self.cancel_msg_id) {
            self.cancel_msg_id = new_msg.id;
            streams.insert(self.cancel_msg_id, ctrl);
        }
    }

    pub async fn finalize(&mut self, ctx: &Context, was_cancelled: bool) {
        if was_cancelled {
            let display = if self.accumulated.is_empty() {
                "*(Cancelled)*".to_string()
            } else {
                format!("{}\n\n*(Cancelled)*", self.accumulated.trim_end())
            };
            let _ = self
                .current_target()
                .edit(ctx, EditMessage::new().content(&display))
                .await;
        } else if !self.accumulated.is_empty() {
            self.flush_edit(ctx).await;
        }
    }
}
