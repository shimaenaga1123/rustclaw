use crate::{
    agent::{Agent, AttachmentInfo, StreamEvent, UserInfo},
    config::Config,
    scheduler::Scheduler,
    utils,
};
use anyhow::Result;
use poise::serenity_prelude::{self as serenity, *};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

const DISCORD_MAX_LEN: usize = 2000;
const MAX_ATTACHMENT_SIZE: u32 = 25 * 1024 * 1024;
const EDIT_INTERVAL_MS: u64 = 1200;

pub struct Data {
    pub agent: Arc<dyn Agent>,
    pub config: Config,
    pub scheduler: Arc<Scheduler>,
    pub owner_id: UserId,
    pub http_client: reqwest::Client,
}

type Error = Box<dyn std::error::Error + Send + Sync>;
type Ctx<'a> = poise::Context<'a, Data, Error>;

async fn build_user_info(ctx: Ctx<'_>) -> UserInfo {
    let author = ctx.author();
    let mut info = UserInfo {
        name: author.name.clone(),
        global_name: author.global_name.clone(),
        id: author.id.get(),
        avatar_url: author.avatar_url(),
        ..Default::default()
    };

    if let Some(member) = ctx.author_member().await {
        info.nickname = member.nick.clone();
        if let Some(guild_id) = ctx.guild_id() {
            if let Some(guild) = guild_id.to_guild_cached(&ctx) {
                info.roles = member
                    .roles
                    .iter()
                    .filter_map(|rid| guild.roles.get(rid).map(|r| r.name.clone()))
                    .collect();
            }
        }
    }

    info
}

fn sanitize_filename(name: &str) -> String {
    name.replace(['/', '\\', '\0', ':', '*', '?', '"', '<', '>', '|'], "_")
        .trim()
        .chars()
        .take(200)
        .collect()
}

async fn download_attachment(
    http_client: &reqwest::Client,
    config: &Config,
    att: &serenity::Attachment,
) -> Option<AttachmentInfo> {
    if att.size > MAX_ATTACHMENT_SIZE {
        warn!(
            "Skipping attachment '{}': too large ({} bytes)",
            att.filename, att.size
        );
        return None;
    }

    let safe = sanitize_filename(&att.filename);
    if safe.is_empty() {
        return None;
    }

    let upload_dir = config.data_dir.join("workspace").join("upload");
    tokio::fs::create_dir_all(&upload_dir).await.ok()?;

    let host_path = upload_dir.join(&safe);
    let final_name = if host_path.exists() {
        let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
        let stem = safe.rfind('.').map(|i| &safe[..i]).unwrap_or(&safe);
        let ext = safe.rfind('.').map(|i| &safe[i..]).unwrap_or("");
        format!("{}_{}{}", stem, ts, ext)
    } else {
        safe
    };

    let host_path = upload_dir.join(&final_name);

    let resp = http_client.get(&att.url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    tokio::fs::write(&host_path, &bytes).await.ok()?;

    info!(
        "Downloaded attachment '{}' ({} bytes)",
        final_name,
        bytes.len()
    );

    Some(AttachmentInfo {
        filename: final_name.clone(),
        container_path: format!("/workspace/upload/{}", final_name),
        size: att.size,
        content_type: att.content_type.clone(),
    })
}

fn get_interaction<'a>(ctx: &'a Ctx<'_>) -> &'a CommandInteraction {
    match ctx {
        poise::Context::Application(app) => app.interaction,
        _ => unreachable!(),
    }
}

#[poise::command(slash_command)]
pub async fn ask(
    ctx: Ctx<'_>,
    #[description = "prompt"] prompt: String,
    #[description = "attachment"] file: Option<serenity::Attachment>,
) -> Result<(), Error> {
    ctx.defer().await?;

    let data = ctx.data();
    let is_owner = ctx.author().id == data.owner_id;
    let channel_id = ctx.channel_id().get();
    let user_info = build_user_info(ctx).await;

    let attachments = match &file {
        Some(att) => match download_attachment(&data.http_client, &data.config, att).await {
            Some(info) => vec![info],
            None => vec![],
        },
        None => vec![],
    };

    let input = if prompt.is_empty() && !attachments.is_empty() {
        "I've attached files. Please check them.".to_string()
    } else {
        prompt
    };

    let (tx, mut rx) = mpsc::channel::<StreamEvent>(128);
    let agent = data.agent.clone();

    let handle = tokio::spawn(async move {
        agent
            .process_streaming(
                &input,
                is_owner,
                Some(channel_id),
                Some(&user_info),
                &attachments,
                tx,
            )
            .await
    });

    let interaction = get_interaction(&ctx);
    let http = ctx.http();

    let mut accumulated = String::new();
    let mut last_edit = Instant::now();
    let edit_interval = Duration::from_millis(EDIT_INTERVAL_MS);

    loop {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Some(StreamEvent::TextDelta(text))) => {
                accumulated.push_str(&text);

                if last_edit.elapsed() >= edit_interval && !accumulated.is_empty() {
                    let preview = if accumulated.len() > 1900 {
                        format!("{}…", &accumulated[..1900])
                    } else {
                        accumulated.clone()
                    };
                    let _ = interaction
                        .edit_response(http, EditInteractionResponse::new().content(&preview))
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
                    let preview = if accumulated.len() > 1900 {
                        format!("{}…", &accumulated[..1900])
                    } else {
                        accumulated.clone()
                    };
                    let _ = interaction
                        .edit_response(http, EditInteractionResponse::new().content(&preview))
                        .await;
                    last_edit = Instant::now();
                }
            }
        }
    }

    match handle.await {
        Ok(Ok(response)) => {
            let final_text = if accumulated.is_empty() {
                &response.text
            } else {
                &accumulated
            };

            if final_text.is_empty() {
                let _ = interaction
                    .edit_response(http, EditInteractionResponse::new().content("(No Content)"))
                    .await;
            } else {
                let chunks = utils::split_message(final_text, DISCORD_MAX_LEN);

                let _ = interaction
                    .edit_response(http, EditInteractionResponse::new().content(&chunks[0]))
                    .await;

                for chunk in &chunks[1..] {
                    let _ = interaction
                        .create_followup(
                            http,
                            CreateInteractionResponseFollowup::new().content(chunk),
                        )
                        .await;
                }
            }

            for file in &response.files {
                if let Ok(attachment) = CreateAttachment::path(&file.path).await {
                    let builder = CreateInteractionResponseFollowup::new().add_file(attachment);
                    if let Err(e) = interaction.create_followup(http, builder).await {
                        error!("Failed to send file '{}': {}", file.filename, e);
                    }
                }

                if file.path.to_string_lossy().contains("/tmp/") {
                    let _ = tokio::fs::remove_file(&file.path).await;
                }
            }

            let _ = interaction
                .create_followup(
                    http,
                    CreateInteractionResponseFollowup::new()
                        .content("✅ Completed!")
                        .ephemeral(true),
                )
                .await;
        }
        Ok(Err(e)) => {
            error!("Agent error: {}", e);
            let _ = interaction
                .edit_response(
                    http,
                    EditInteractionResponse::new().content("An error occurred while processing"),
                )
                .await;
        }
        Err(e) => {
            error!("Task join error: {}", e);
            let _ = interaction
                .edit_response(
                    http,
                    EditInteractionResponse::new().content("An error occurred while processing"),
                )
                .await;
        }
    }

    Ok(())
}

pub async fn setup(config: Config, agent: Arc<dyn Agent>, scheduler: Arc<Scheduler>) -> Result<()> {
    let owner_id = UserId::new(config.owner_id);
    let token = config.discord_token.clone();

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![ask()],
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            let scheduler = scheduler.clone();
            let config = config.clone();
            let agent = agent.clone();
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                scheduler.set_discord_http(ctx.http.clone()).await;
                info!("Registered /ask slash command");
                Ok(Data {
                    agent,
                    config,
                    scheduler,
                    owner_id,
                    http_client: reqwest::Client::new(),
                })
            })
        })
        .build();

    let intents = GatewayIntents::GUILDS;

    let mut client = serenity::ClientBuilder::new(&token, intents)
        .framework(framework)
        .await?;

    client.start().await?;
    Ok(())
}
