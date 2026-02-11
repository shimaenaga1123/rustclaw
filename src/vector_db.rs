use crate::embeddings::EmbeddingService;
use crate::entity::{conversations, important};
use anyhow::{Context, Result};
use sea_orm::*;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::info;
use usearch::Index;
use usearch::ffi::{IndexOptions, MetricKind, ScalarKind};

const INDEX_FILE: &str = "conversations.usearch";

pub struct VectorDb {
    db_url: String,
    index: Arc<Mutex<Index>>,
    embeddings: Arc<dyn EmbeddingService>,
    index_path: PathBuf,
}

impl VectorDb {
    pub async fn new(data_dir: &Path, embeddings: Arc<dyn EmbeddingService>) -> Result<Arc<Self>> {
        let db_path = data_dir.join("memory.db");
        std::fs::create_dir_all(data_dir)?;
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

        tokio::task::spawn_blocking({
            let db_url = db_url.clone();
            move || -> Result<()> {
                let db = Database::connect(&db_url)?;

                db.get_schema_builder()
                    .register(conversations::Entity)
                    .register(important::Entity)
                    .apply(&db)?;

                Ok(())
            }
        })
        .await??;

        let index_path = data_dir.join(INDEX_FILE);
        let options = IndexOptions {
            dimensions: embeddings.dimensions(),
            metric: MetricKind::Cos,
            quantization: ScalarKind::F16,
            ..Default::default()
        };
        let index = Index::new(&options).context("Failed to create usearch index")?;

        if index_path.exists() {
            index
                .load(index_path.to_str().unwrap())
                .context("Failed to load usearch index")?;
            info!("Loaded usearch index ({} vectors)", index.size());
        } else {
            index.reserve(1000).context("Failed to reserve index")?;
        }

        let instance = Arc::new(Self {
            db_url,
            index: Arc::new(Mutex::new(index)),
            embeddings,
            index_path,
        });

        info!("VectorDB ready (usearch + rusqlite)");
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

        let record = conversations::ActiveModel {
            rowid: NotSet,
            id: Set(id),
            author: Set(author.to_string()),
            user_input: Set(user_input.to_string()),
            assistant_response: Set(assistant_response.to_string()),
            timestamp_us: Set(now),
        };

        let index = self.index.clone();
        let index_path = self.index_path.clone();
        let db_url = self.db_url.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let db = Database::connect(&db_url)?;
            let result = conversations::Entity::insert(record).exec(&db)?;
            let rowid = result.last_insert_id as u64;

            let idx = index.lock().unwrap();
            if idx.size() + 1 >= idx.capacity() {
                idx.reserve(idx.capacity() + 1000)
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
        let db_url = self.db_url.clone();

        tokio::task::spawn_blocking(move || -> Result<Vec<ConversationTurn>> {
            let db = Database::connect(&db_url)?;
            let rows = conversations::Entity::find()
                .order_by_desc(conversations::Column::TimestampUs)
                .limit(n as u64)
                .all(&db)?;

            let mut turns: Vec<ConversationTurn> = rows.into_iter().map(|r| r.into()).collect();
            turns.reverse();
            Ok(turns)
        })
        .await?
    }

    pub async fn search_turns(
        &self,
        query: &str,
        top_k: usize,
        exclude_ids: &[String],
    ) -> Result<Vec<ConversationTurn>> {
        let embedding = self.embeddings.embed_query(query).await?;
        let fetch_n = top_k + exclude_ids.len() + 5;
        let exclude = exclude_ids.to_vec();

        let index = self.index.clone();
        let db_url = self.db_url.clone();

        tokio::task::spawn_blocking(move || -> Result<Vec<ConversationTurn>> {
            let db = Database::connect(&db_url)?;
            let results = {
                let idx = index.lock().unwrap();
                idx.search(&embedding, fetch_n)
                    .map_err(|e| anyhow::anyhow!("{}", e))?
            };

            let rowids: Vec<i64> = results.keys.iter().map(|k| *k as i64).collect();
            if rowids.is_empty() {
                return Ok(Vec::new());
            }

            let rows = conversations::Entity::find()
                .filter(conversations::Column::Rowid.is_in(rowids))
                .all(&db)?;

            let mut turns: Vec<ConversationTurn> = rows
                .into_iter()
                .map(|r| r.into())
                .filter(|t: &ConversationTurn| !exclude.contains(&t.id))
                .collect();

            turns.sort_by(|a, b| a.timestamp_micros.cmp(&b.timestamp_micros));
            turns.truncate(top_k);
            Ok(turns)
        })
        .await?
    }

    pub async fn add_important(&self, content: &str) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let now = chrono::Utc::now().timestamp_micros();

        let record = important::ActiveModel {
            rowid: NotSet,
            id: Set(id.clone()),
            content: Set(content.to_string()),
            timestamp_us: Set(now),
        };

        let db_url = self.db_url.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let db = Database::connect(&db_url)?;
            important::Entity::insert(record).exec(&db)?;
            Ok(())
        })
        .await??;

        info!("Added important entry: {}", id);
        Ok(id)
    }

    pub async fn list_important(&self) -> Result<Vec<ImportantEntry>> {
        let db_url = self.db_url.clone();

        tokio::task::spawn_blocking(move || -> Result<Vec<ImportantEntry>> {
            let db = Database::connect(&db_url)?;
            let rows = important::Entity::find()
                .order_by_asc(important::Column::TimestampUs)
                .all(&db)?;

            Ok(rows.into_iter().map(|r| r.into()).collect())
        })
        .await?
    }

    pub async fn delete_important(&self, id: &str) -> Result<bool> {
        let db_url = self.db_url.clone();
        let id = id.to_string();

        let affected = tokio::task::spawn_blocking(move || -> Result<u64> {
            let db = Database::connect(&db_url)?;
            let result = important::Entity::delete_many()
                .filter(important::Column::Id.eq(&id))
                .exec(&db)?;
            Ok(result.rows_affected)
        })
        .await??;

        info!("Deleted important entry");
        Ok(affected > 0)
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

impl From<conversations::Model> for ConversationTurn {
    fn from(r: conversations::Model) -> Self {
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

impl From<important::Model> for ImportantEntry {
    fn from(r: important::Model) -> Self {
        Self {
            id: r.id,
            content: r.content,
            timestamp_micros: r.timestamp_us,
        }
    }
}
