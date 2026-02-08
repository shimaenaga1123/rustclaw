use crate::embeddings::{EMBEDDING_DIM, EmbeddingService};
use anyhow::{Context, Result};
use sqlx::FromRow;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::info;
use usearch::Index;
use usearch::ffi::{IndexOptions, MetricKind, ScalarKind};

const INDEX_FILE: &str = "conversations.usearch";

#[derive(FromRow)]
struct ConversationRow {
    id: String,
    author: String,
    user_input: String,
    assistant_response: String,
    timestamp_us: i64,
}

#[derive(FromRow)]
struct ImportantRow {
    id: String,
    content: String,
    timestamp_us: i64,
}

pub struct VectorDb {
    pool: SqlitePool,
    index: Arc<Mutex<Index>>,
    embeddings: Arc<EmbeddingService>,
    index_path: PathBuf,
}

impl VectorDb {
    pub async fn new(data_dir: &Path, embeddings: Arc<EmbeddingService>) -> Result<Arc<Self>> {
        let db_path = data_dir.join("memory.db");
        std::fs::create_dir_all(data_dir)?;
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await
            .context("Failed to connect to SQLite")?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS conversations (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT UNIQUE NOT NULL,
                author TEXT NOT NULL,
                user_input TEXT NOT NULL,
                assistant_response TEXT NOT NULL,
                timestamp_us INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS important (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT UNIQUE NOT NULL,
                content TEXT NOT NULL,
                timestamp_us INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        let index_path = data_dir.join(INDEX_FILE);
        let options = IndexOptions {
            dimensions: EMBEDDING_DIM as usize,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            ..Default::default()
        };
        let index = Index::new(&options).context("Failed to create usearch index")?;

        if index_path.exists() {
            index
                .load(index_path.to_str().unwrap())
                .context("Failed to load usearch index")?;
            info!("Loaded usearch index ({} vectors)", index.size());
        } else {
            index.reserve(10000).context("Failed to reserve index")?;
        }

        let instance = Arc::new(Self {
            pool,
            index: Arc::new(Mutex::new(index)),
            embeddings,
            index_path,
        });

        info!("VectorDB ready (usearch + SQLite)");
        Ok(instance)
    }

    pub async fn add_turn(
        &self,
        author: &str,
        user_input: &str,
        assistant_response: &str,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_micros();

        let combined = format!(
            "{}: {}\nAssistant: {}",
            author, user_input, assistant_response
        );
        let embedding = self.embeddings.embed_passage(&combined).await?;

        let result = sqlx::query(
            "INSERT INTO conversations (id, author, user_input, assistant_response, timestamp_us) VALUES (?, ?, ?, ?, ?)",
        )
            .bind(&id)
            .bind(author)
            .bind(user_input)
            .bind(assistant_response)
            .bind(now)
            .execute(&self.pool)
            .await?;

        let rowid = result.last_insert_rowid() as u64;

        let index = self.index.clone();
        let index_path = self.index_path.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let idx = index.lock().unwrap();
            if idx.size() + 1 >= idx.capacity() {
                idx.reserve(idx.capacity() + 10000)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
            }
            idx.add(rowid, &embedding)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            idx.save(index_path.to_str().unwrap())
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(())
        })
        .await??;

        Ok(())
    }

    pub async fn recent_turns(&self, n: usize) -> Result<Vec<ConversationTurn>> {
        let rows = sqlx::query_as::<_, ConversationRow>(
            "SELECT id, author, user_input, assistant_response, timestamp_us FROM conversations ORDER BY timestamp_us DESC LIMIT ?",
        )
            .bind(n as i64)
            .fetch_all(&self.pool)
            .await?;

        let mut turns: Vec<ConversationTurn> = rows.into_iter().map(|r| r.into()).collect();
        turns.reverse();
        Ok(turns)
    }

    pub async fn search_turns(
        &self,
        query: &str,
        top_k: usize,
        exclude_ids: &[String],
    ) -> Result<Vec<ConversationTurn>> {
        let embedding = self.embeddings.embed_query(query).await?;
        let fetch_n = top_k + exclude_ids.len() + 5;

        let index = self.index.clone();
        let results = tokio::task::spawn_blocking(move || {
            let idx = index.lock().unwrap();
            idx.search(&embedding, fetch_n)
        })
        .await?
        .map_err(|e| anyhow::anyhow!("{}", e))?;

        let rowids: Vec<i64> = results.keys.iter().map(|k| *k as i64).collect();
        if rowids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: String = rowids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT id, author, user_input, assistant_response, timestamp_us \
             FROM conversations WHERE rowid IN ({})",
            placeholders
        );

        let mut q = sqlx::query_as::<_, ConversationRow>(&sql);
        for rowid in &rowids {
            q = q.bind(rowid);
        }
        let rows = q.fetch_all(&self.pool).await?;

        let mut turns: Vec<ConversationTurn> = rows
            .into_iter()
            .map(|r| r.into())
            .filter(|t: &ConversationTurn| !exclude_ids.contains(&t.id))
            .collect();

        turns.sort_by(|a, b| a.timestamp_micros.cmp(&b.timestamp_micros));
        turns.truncate(top_k);
        Ok(turns)
    }

    pub async fn add_important(&self, content: &str) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let now = chrono::Utc::now().timestamp_micros();

        sqlx::query("INSERT INTO important (id, content, timestamp_us) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(content)
            .bind(now)
            .execute(&self.pool)
            .await?;

        info!("Added important entry: {}", id);
        Ok(id)
    }

    pub async fn list_important(&self) -> Result<Vec<ImportantEntry>> {
        let rows = sqlx::query_as::<_, ImportantRow>(
            "SELECT id, content, timestamp_us FROM important ORDER BY timestamp_us ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    pub async fn delete_important(&self, id: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM important WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        info!("Deleted important entry: {}", id);
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_important_context(&self) -> Result<String> {
        let entries = self.list_important().await?;
        if entries.is_empty() {
            return Ok(String::new());
        }

        let mut context = String::from("# Important Facts\n\n");
        for entry in &entries {
            context.push_str(&format!("- {}\n", entry.content));
        }
        Ok(context)
    }
}

#[derive(Debug, Clone)]
pub struct ConversationTurn {
    pub id: String,
    pub author: String,
    pub user_input: String,
    pub assistant_response: String,
    pub timestamp_micros: i64,
}

impl ConversationTurn {
    pub fn format_for_context(&self) -> String {
        format!(
            "{}: {}\nAssistant: {}",
            self.author, self.user_input, self.assistant_response
        )
    }
}

impl From<ConversationRow> for ConversationTurn {
    fn from(r: ConversationRow) -> Self {
        Self {
            id: r.id,
            author: r.author,
            user_input: r.user_input,
            assistant_response: r.assistant_response,
            timestamp_micros: r.timestamp_us,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImportantEntry {
    pub id: String,
    pub content: String,
    pub timestamp_micros: i64,
}

impl From<ImportantRow> for ImportantEntry {
    fn from(r: ImportantRow) -> Self {
        Self {
            id: r.id,
            content: r.content,
            timestamp_micros: r.timestamp_us,
        }
    }
}
