use anyhow::Result;
use chrono::Utc;
use std::{path::PathBuf, sync::Arc};
use tokio::{fs, sync::RwLock};

pub struct MemoryManager {
    data_dir: PathBuf,
    recent: Arc<RwLock<Vec<String>>>,
    long_term: Arc<RwLock<String>>,
}

impl MemoryManager {
    pub async fn new(data_dir: &PathBuf) -> Result<Arc<Self>> {
        fs::create_dir_all(data_dir).await?;

        let recent_path = data_dir.join("recent.md");
        let memory_path = data_dir.join("memory.md");

        let recent_content = fs::read_to_string(&recent_path).await.unwrap_or_default();
        let recent: Vec<String> = recent_content
            .lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect();

        let long_term = fs::read_to_string(&memory_path).await.unwrap_or_default();

        Ok(Arc::new(Self {
            data_dir: data_dir.clone(),
            recent: Arc::new(RwLock::new(recent)),
            long_term: Arc::new(RwLock::new(long_term)),
        }))
    }

    pub async fn add_message(&self, author: &str, content: &str) -> Result<()> {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S");
        let entry = format!("[{}] {}: {}", timestamp, author, content);

        let mut recent = self.recent.write().await;
        recent.push(entry);

        self.save_recent(&recent).await?;

        Ok(())
    }

    pub async fn add_assistant_message(&self, content: &str) -> Result<()> {
        self.add_message("Assistant", content).await
    }

    pub async fn get_context(&self) -> Result<String> {
        let recent = self.recent.read().await;
        let long_term = self.long_term.read().await;

        let mut context = String::new();

        if !long_term.is_empty() {
            context.push_str("# Long-term Memory\n\n");
            context.push_str(&long_term);
            context.push_str("\n\n");
        }

        if !recent.is_empty() {
            context.push_str("# Recent Conversations\n\n");
            context.push_str(&recent.join("\n"));
        }

        Ok(context)
    }

    pub async fn compress_memory(&self, summary: &str) -> Result<()> {
        let mut long_term = self.long_term.write().await;
        let mut recent = self.recent.write().await;

        let archive_path = self.data_dir.join("conversations");
        fs::create_dir_all(&archive_path).await?;

        let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
        let archive_file = archive_path.join(format!("{}.md", timestamp));

        fs::write(&archive_file, recent.join("\n")).await?;

        long_term.push_str(&format!("\n\n## {}\n{}", timestamp, summary));
        self.save_long_term(&long_term).await?;

        recent.clear();
        self.save_recent(&recent).await?;

        Ok(())
    }

    pub async fn add_to_long_term(&self, content: &str) -> Result<()> {
        let mut long_term = self.long_term.write().await;
        long_term.push_str(&format!("\n\n{}", content));
        self.save_long_term(&long_term).await
    }

    async fn save_recent(&self, recent: &[String]) -> Result<()> {
        let path = self.data_dir.join("recent.md");
        fs::write(path, recent.join("\n")).await?;
        Ok(())
    }

    async fn save_long_term(&self, content: &str) -> Result<()> {
        let path = self.data_dir.join("memory.md");
        fs::write(path, content).await?;
        Ok(())
    }
}
