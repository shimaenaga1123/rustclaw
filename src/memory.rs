use anyhow::Result;
use chrono::Utc;
use std::{path::PathBuf, sync::Arc};
use tokio::{fs, sync::RwLock};
use tracing::info;

const MAX_RECENT_MESSAGES: usize = 200;
const MAX_LONG_TERM_ENTRIES: usize = 100;

pub struct MemoryManager {
    data_dir: PathBuf,
    recent: Arc<RwLock<Vec<String>>>,
    long_term: Arc<RwLock<Vec<String>>>,
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

        let long_term_content = fs::read_to_string(&memory_path).await.unwrap_or_default();
        let long_term: Vec<String> = long_term_content
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

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

        if recent.len() > MAX_RECENT_MESSAGES {
            let overflow = recent.len() - MAX_RECENT_MESSAGES;
            let drained: Vec<String> = recent.drain(..overflow).collect();

            self.archive_messages(&drained).await?;
            info!("Auto-trimmed {} old messages from recent memory", overflow);
        }

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
            for entry in long_term.iter() {
                context.push_str("- ");
                context.push_str(entry);
                context.push('\n');
            }
            context.push('\n');
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

        self.archive_messages(&recent).await?;

        let timestamp = Utc::now().format("%Y-%m-%d %H:%M");
        let summary_entry = format!("[{}] {}", timestamp, summary.trim());
        long_term.push(summary_entry);

        if long_term.len() > MAX_LONG_TERM_ENTRIES {
            let overflow = long_term.len() - MAX_LONG_TERM_ENTRIES;
            long_term.drain(..overflow);
            info!("Trimmed {} old entries from long-term memory", overflow);
        }

        self.save_long_term(&long_term).await?;

        recent.clear();
        self.save_recent(&recent).await?;

        Ok(())
    }

    pub async fn add_to_long_term(&self, content: &str) -> Result<()> {
        let mut long_term = self.long_term.write().await;

        let content_normalized = content.trim().to_lowercase();

        let is_duplicate = long_term.iter().any(|entry| {
            let existing = entry
                .find(']')
                .map(|i| &entry[i + 1..])
                .unwrap_or(entry)
                .trim()
                .to_lowercase();

            existing == content_normalized
                || (existing.len() > 10
                    && content_normalized.len() > 10
                    && (existing.contains(&content_normalized)
                        || content_normalized.contains(&existing)))
        });

        if is_duplicate {
            info!("Skipping duplicate long-term memory entry");
            return Ok(());
        }

        let timestamp = Utc::now().format("%Y-%m-%d %H:%M");
        long_term.push(format!("[{}] {}", timestamp, content.trim()));

        if long_term.len() > MAX_LONG_TERM_ENTRIES {
            long_term.remove(0);
        }

        self.save_long_term(&long_term).await
    }

    pub async fn flush(&self) -> Result<()> {
        let recent = self.recent.read().await;
        self.save_recent(&recent).await?;

        let long_term = self.long_term.read().await;
        self.save_long_term(&long_term).await?;

        info!("Memory flushed to disk");
        Ok(())
    }

    async fn archive_messages(&self, messages: &[String]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }

        let archive_path = self.data_dir.join("conversations");
        fs::create_dir_all(&archive_path).await?;

        let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
        let archive_file = archive_path.join(format!("{}.md", timestamp));

        fs::write(&archive_file, messages.join("\n")).await?;
        info!(
            "Archived {} messages to {}",
            messages.len(),
            archive_file.display()
        );

        Ok(())
    }

    async fn save_recent(&self, recent: &[String]) -> Result<()> {
        let path = self.data_dir.join("recent.md");
        fs::write(path, recent.join("\n")).await?;
        Ok(())
    }

    async fn save_long_term(&self, entries: &[String]) -> Result<()> {
        let path = self.data_dir.join("memory.md");
        fs::write(path, entries.join("\n\n")).await?;
        Ok(())
    }
}
