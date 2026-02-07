use crate::agent::RigAgent;
use crate::utils;
use anyhow::Result;
use chrono_tz::Tz;
use iana_time_zone::get_timezone;
use serde::{Deserialize, Serialize};
use serenity::all::{ChannelId, CreateAttachment, CreateMessage, Http};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::{Mutex, RwLock};
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info};
use uuid::Uuid;

const DISCORD_MAX_LEN: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub cron_expr: String,
    pub prompt: String,
    pub description: String,
    #[serde(default)]
    pub is_owner: bool,
    #[serde(default)]
    pub discord_channel_id: Option<u64>,
}

pub struct Scheduler {
    agent: Arc<RigAgent>,
    scheduler: Mutex<JobScheduler>,
    tasks: Arc<RwLock<HashMap<String, ScheduledTask>>>,
    job_ids: Arc<RwLock<HashMap<String, uuid::Uuid>>>,
    data_path: PathBuf,
    discord_http: Arc<RwLock<Option<Arc<Http>>>>,
}

impl Scheduler {
    fn normalize_cron_expr(cron_expr: &str) -> String {
        let parts: Vec<&str> = cron_expr.split_whitespace().collect();
        if parts.len() == 5 {
            format!("0 {}", cron_expr.trim())
        } else {
            cron_expr.to_string()
        }
    }

    pub async fn new(data_dir: &PathBuf, agent: Arc<RigAgent>) -> Result<Arc<Self>> {
        let scheduler = JobScheduler::new().await?;
        let data_path = data_dir.join("schedules.json");

        let instance = Arc::new(Self {
            agent,
            scheduler: Mutex::new(scheduler),
            tasks: Arc::new(RwLock::new(HashMap::new())),
            job_ids: Arc::new(RwLock::new(HashMap::new())),
            data_path,
            discord_http: Arc::new(RwLock::new(None)),
        });

        instance.load_tasks().await?;

        Ok(instance)
    }

    pub async fn start(self: &Arc<Self>) -> Result<()> {
        let tasks = self.tasks.read().await;
        for task in tasks.values() {
            if let Err(e) = self.register_job(task).await {
                error!("Failed to register job {}: {}", task.id, e);
            }
        }
        drop(tasks);

        self.scheduler.lock().await.start().await?;
        info!("Scheduler started");
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.scheduler.lock().await.shutdown().await?;
        self.save_tasks().await?;
        info!("Scheduler shut down");
        Ok(())
    }

    pub async fn set_discord_http(&self, http: Arc<Http>) {
        *self.discord_http.write().await = Some(http);
    }

    async fn register_job(&self, task: &ScheduledTask) -> Result<()> {
        let agent = self.agent.clone();
        let prompt = task.prompt.clone();
        let task_id = task.id.clone();
        let is_owner = task.is_owner;
        let discord_channel_id = task.discord_channel_id;
        let discord_http = self.discord_http.clone();
        let timezone: Tz = get_timezone()?.parse()?;

        let job = Job::new_async_tz(task.cron_expr.as_str(), timezone, move |_uuid, _l| {
            let agent = agent.clone();
            let prompt = prompt.clone();
            let task_id = task_id.clone();
            let discord_http = discord_http.clone();
            Box::pin(async move {
                info!("Running scheduled task: {}", task_id);
                let (tx, mut rx) = tokio::sync::mpsc::channel(128);
                let agent_clone = agent.clone();
                let prompt_clone = prompt.clone();
                let handle = tokio::spawn(async move {
                    agent_clone
                        .process_streaming(
                            &prompt_clone,
                            is_owner,
                            discord_channel_id,
                            None,
                            &[],
                            tx,
                        )
                        .await
                });
                while rx.recv().await.is_some() {}
                match handle
                    .await
                    .unwrap_or_else(|e| Err(anyhow::anyhow!("{}", e)))
                {
                    Ok(response) => {
                        if let Some(channel_id) = discord_channel_id
                            && let Some(http) = discord_http.read().await.as_ref()
                        {
                            let channel = ChannelId::new(channel_id);

                            for chunk in utils::split_message(&response.text, DISCORD_MAX_LEN) {
                                if let Err(e) = channel.say(http, &chunk).await {
                                    error!(
                                        "Failed to send scheduled task result to Discord: {}",
                                        e
                                    );
                                }
                            }

                            for file in &response.files {
                                match CreateAttachment::path(&file.path).await {
                                    Ok(attachment) => {
                                        let builder = CreateMessage::new().add_file(attachment);
                                        if let Err(e) = channel.send_message(http, builder).await {
                                            error!(
                                                "Failed to send scheduled file '{}': {}",
                                                file.filename, e
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        error!(
                                            "Failed to create attachment for '{}': {}",
                                            file.filename, e
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Scheduled task {} failed: {}", task_id, e);
                    }
                }
            })
        })?;

        let job_id = self.scheduler.lock().await.add(job).await?;
        self.job_ids.write().await.insert(task.id.clone(), job_id);

        Ok(())
    }

    pub async fn add_task(
        self: &Arc<Self>,
        cron_expr: &str,
        prompt: &str,
        description: &str,
        is_owner: bool,
        discord_channel_id: Option<u64>,
    ) -> Result<String> {
        let cron_expr = Self::normalize_cron_expr(cron_expr);
        let cron_expr = cron_expr.as_str();
        let timezone: Tz = get_timezone()?.parse()?;

        Job::new_async_tz(cron_expr, timezone, |_, _| Box::pin(async {}))?;

        let task = ScheduledTask {
            id: Uuid::new_v4().to_string()[..8].to_string(),
            cron_expr: cron_expr.to_string(),
            prompt: prompt.to_string(),
            description: description.to_string(),
            is_owner,
            discord_channel_id,
        };

        let task_id = task.id.clone();

        self.register_job(&task).await?;
        self.tasks.write().await.insert(task.id.clone(), task);
        self.save_tasks().await?;

        info!("Added scheduled task: {}", task_id);
        Ok(task_id)
    }

    pub async fn remove_task(&self, task_id: &str) -> Result<bool> {
        let mut tasks = self.tasks.write().await;
        let mut job_ids = self.job_ids.write().await;

        if let Some(_task) = tasks.remove(task_id) {
            if let Some(job_id) = job_ids.remove(task_id) {
                self.scheduler.lock().await.remove(&job_id).await?;
            }
            drop(tasks);
            drop(job_ids);
            self.save_tasks().await?;
            info!("Removed scheduled task: {}", task_id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn list_tasks(&self) -> Vec<ScheduledTask> {
        self.tasks.read().await.values().cloned().collect()
    }

    async fn load_tasks(&self) -> Result<()> {
        if self.data_path.exists() {
            let content = fs::read_to_string(&self.data_path).await?;
            let tasks: Vec<ScheduledTask> = serde_json::from_str(&content)?;
            let mut task_map = self.tasks.write().await;
            for task in tasks {
                task_map.insert(task.id.clone(), task);
            }
            info!("Loaded {} scheduled tasks", task_map.len());
        }
        Ok(())
    }

    async fn save_tasks(&self) -> Result<()> {
        let tasks: Vec<ScheduledTask> = self.tasks.read().await.values().cloned().collect();
        let content = serde_json::to_string_pretty(&tasks)?;
        fs::write(&self.data_path, content).await?;
        Ok(())
    }
}
