use crate::agent::RigAgent;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub cron_expr: String,
    pub prompt: String,
    pub description: String,
    #[serde(default)]
    pub is_owner: bool,
}

pub struct Scheduler {
    agent: Arc<RigAgent>,
    scheduler: JobScheduler,
    tasks: Arc<RwLock<HashMap<String, ScheduledTask>>>,
    job_ids: Arc<RwLock<HashMap<String, uuid::Uuid>>>,
    data_path: PathBuf,
}

impl Scheduler {
    pub async fn new(data_dir: &PathBuf, agent: Arc<RigAgent>) -> Result<Arc<Self>> {
        let scheduler = JobScheduler::new().await?;
        let data_path = data_dir.join("schedules.json");

        let instance = Arc::new(Self {
            agent,
            scheduler,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            job_ids: Arc::new(RwLock::new(HashMap::new())),
            data_path,
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

        self.scheduler.start().await?;
        info!("Scheduler started");
        Ok(())
    }

    async fn register_job(&self, task: &ScheduledTask) -> Result<()> {
        let agent = self.agent.clone();
        let prompt = task.prompt.clone();
        let task_id = task.id.clone();
        let is_owner = task.is_owner;

        let job = Job::new_async(task.cron_expr.as_str(), move |_uuid, _l| {
            let agent = agent.clone();
            let prompt = prompt.clone();
            let task_id = task_id.clone();
            Box::pin(async move {
                info!("Running scheduled task: {}", task_id);
                if let Err(e) = agent.process(&prompt, is_owner).await {
                    error!("Scheduled task {} failed: {}", task_id, e);
                }
            })
        })?;

        let job_id = self.scheduler.add(job).await?;
        self.job_ids.write().await.insert(task.id.clone(), job_id);

        Ok(())
    }

    pub async fn add_task(
        self: &Arc<Self>,
        cron_expr: &str,
        prompt: &str,
        description: &str,
        is_owner: bool,
    ) -> Result<String> {
        Job::new_async(cron_expr, |_, _| Box::pin(async {}))?;

        let task = ScheduledTask {
            id: Uuid::new_v4().to_string()[..8].to_string(),
            cron_expr: cron_expr.to_string(),
            prompt: prompt.to_string(),
            description: description.to_string(),
            is_owner,
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
                self.scheduler.remove(&job_id).await?;
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
