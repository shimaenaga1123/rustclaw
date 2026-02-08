use crate::embeddings::{EMBEDDING_DIM, EmbeddingService};
use anyhow::{Context, Result};
use arrow_array::{
    ArrayRef, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray,
    TimestampMicrosecondArray,
};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use chrono::Utc;
use futures_util::TryStreamExt;
use lancedb::{
    Connection, connect,
    query::{ExecutableQuery, QueryBase},
};
use std::path::Path;
use std::sync::Arc;
use tracing::info;

const LONG_TERM_TABLE: &str = "long_term_memory";
const IMPORTANT_TABLE: &str = "important";

pub struct VectorDb {
    db: Connection,
    embeddings: Arc<EmbeddingService>,
}

impl VectorDb {
    pub async fn new(data_dir: &Path, embeddings: Arc<EmbeddingService>) -> Result<Arc<Self>> {
        let db_path = data_dir.join("lancedb");
        std::fs::create_dir_all(&db_path)?;

        let db = connect(db_path.to_str().unwrap())
            .execute()
            .await
            .context("Failed to open LanceDB")?;

        let instance = Arc::new(Self { db, embeddings });
        instance.ensure_tables().await?;

        info!("VectorDB ready");
        Ok(instance)
    }

    fn long_term_schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("author", DataType::Utf8, false),
            Field::new("user_input", DataType::Utf8, false),
            Field::new("assistant_response", DataType::Utf8, false),
            Field::new(
                "timestamp",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    EMBEDDING_DIM,
                ),
                true,
            ),
        ]))
    }

    fn important_schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new(
                "timestamp",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    EMBEDDING_DIM,
                ),
                true,
            ),
        ]))
    }

    async fn ensure_tables(&self) -> Result<()> {
        let existing: Vec<String> = self.db.table_names().execute().await?;

        if !existing.iter().any(|n| n == LONG_TERM_TABLE) {
            let schema = Self::long_term_schema();
            let batch = RecordBatch::new_empty(schema.clone());
            let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
            self.db
                .create_table(LONG_TERM_TABLE, Box::new(reader))
                .execute()
                .await
                .context("Failed to create long_term_memory table")?;
            info!("Created table: {}", LONG_TERM_TABLE);
        }

        if !existing.iter().any(|n| n == IMPORTANT_TABLE) {
            let schema = Self::important_schema();
            let batch = RecordBatch::new_empty(schema.clone());
            let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
            self.db
                .create_table(IMPORTANT_TABLE, Box::new(reader))
                .execute()
                .await
                .context("Failed to create important table")?;
            info!("Created table: {}", IMPORTANT_TABLE);
        }

        Ok(())
    }

    pub async fn add_turn(
        &self,
        author: &str,
        user_input: &str,
        assistant_response: &str,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().timestamp_micros();

        let combined = format!(
            "{}: {}\nAssistant: {}",
            author, user_input, assistant_response
        );
        let embedding = self.embeddings.embed_passage(&combined).await?;

        let schema = Self::long_term_schema();
        let vector_array = make_fixed_list_array(&embedding, EMBEDDING_DIM);

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![id.as_str()])) as ArrayRef,
                Arc::new(StringArray::from(vec![author])),
                Arc::new(StringArray::from(vec![user_input])),
                Arc::new(StringArray::from(vec![assistant_response])),
                Arc::new(TimestampMicrosecondArray::from(vec![now])),
                vector_array,
            ],
        )?;

        let table = self.db.open_table(LONG_TERM_TABLE).execute().await?;
        let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table.add(Box::new(reader)).execute().await?;

        Ok(())
    }

    pub async fn recent_turns(&self, n: usize) -> Result<Vec<ConversationTurn>> {
        let table = self.db.open_table(LONG_TERM_TABLE).execute().await?;

        let batches: Vec<RecordBatch> = table
            .query()
            .select(lancedb::query::Select::columns(&[
                "id",
                "author",
                "user_input",
                "assistant_response",
                "timestamp",
            ]))
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut turns: Vec<ConversationTurn> = Vec::new();
        for batch in &batches {
            let ids = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let authors = batch
                .column_by_name("author")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let inputs = batch
                .column_by_name("user_input")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let responses = batch
                .column_by_name("assistant_response")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let timestamps = batch
                .column_by_name("timestamp")
                .unwrap()
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();

            for i in 0..batch.num_rows() {
                turns.push(ConversationTurn {
                    id: ids.value(i).to_string(),
                    author: authors.value(i).to_string(),
                    user_input: inputs.value(i).to_string(),
                    assistant_response: responses.value(i).to_string(),
                    timestamp_micros: timestamps.value(i),
                });
            }
        }

        turns.sort_by(|a, b| b.timestamp_micros.cmp(&a.timestamp_micros));
        turns.truncate(n);
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
        let table = self.db.open_table(LONG_TERM_TABLE).execute().await?;

        let fetch_n = top_k + exclude_ids.len();

        let batches: Vec<RecordBatch> = table
            .query()
            .nearest_to(embedding)?
            .limit(fetch_n)
            .select(lancedb::query::Select::columns(&[
                "id",
                "author",
                "user_input",
                "assistant_response",
                "timestamp",
            ]))
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut turns = Vec::new();
        for batch in &batches {
            let ids = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let authors = batch
                .column_by_name("author")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let inputs = batch
                .column_by_name("user_input")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let responses = batch
                .column_by_name("assistant_response")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let timestamps = batch
                .column_by_name("timestamp")
                .unwrap()
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();

            for i in 0..batch.num_rows() {
                let id = ids.value(i).to_string();
                if exclude_ids.contains(&id) {
                    continue;
                }
                turns.push(ConversationTurn {
                    id,
                    author: authors.value(i).to_string(),
                    user_input: inputs.value(i).to_string(),
                    assistant_response: responses.value(i).to_string(),
                    timestamp_micros: timestamps.value(i),
                });
                if turns.len() >= top_k {
                    break;
                }
            }
            if turns.len() >= top_k {
                break;
            }
        }

        turns.sort_by(|a, b| a.timestamp_micros.cmp(&b.timestamp_micros));
        Ok(turns)
    }

    pub async fn add_important(&self, content: &str) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let now = Utc::now().timestamp_micros();
        let embedding = self.embeddings.embed_passage(content).await?;

        let schema = Self::important_schema();
        let vector_array = make_fixed_list_array(&embedding, EMBEDDING_DIM);

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![id.as_str()])) as ArrayRef,
                Arc::new(StringArray::from(vec![content])),
                Arc::new(TimestampMicrosecondArray::from(vec![now])),
                vector_array,
            ],
        )?;

        let table = self.db.open_table(IMPORTANT_TABLE).execute().await?;
        let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table.add(Box::new(reader)).execute().await?;

        info!("Added important entry: {}", id);
        Ok(id)
    }

    pub async fn list_important(&self) -> Result<Vec<ImportantEntry>> {
        let table = self.db.open_table(IMPORTANT_TABLE).execute().await?;

        let batches: Vec<RecordBatch> = table
            .query()
            .select(lancedb::query::Select::columns(&[
                "id",
                "content",
                "timestamp",
            ]))
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut entries = Vec::new();
        for batch in &batches {
            let ids = batch
                .column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let contents = batch
                .column_by_name("content")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let timestamps = batch
                .column_by_name("timestamp")
                .unwrap()
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();

            for i in 0..batch.num_rows() {
                entries.push(ImportantEntry {
                    id: ids.value(i).to_string(),
                    content: contents.value(i).to_string(),
                    timestamp_micros: timestamps.value(i),
                });
            }
        }

        entries.sort_by(|a, b| a.timestamp_micros.cmp(&b.timestamp_micros));
        Ok(entries)
    }

    pub async fn delete_important(&self, id: &str) -> Result<bool> {
        let table = self.db.open_table(IMPORTANT_TABLE).execute().await?;
        let filter = format!("id = '{}'", id.replace('\'', "''"));
        table.delete(&filter).await?;
        info!("Deleted important entry: {}", id);
        Ok(true)
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

#[derive(Debug, Clone)]
pub struct ImportantEntry {
    pub id: String,
    pub content: String,
    pub timestamp_micros: i64,
}

fn make_fixed_list_array(values: &[f32], dim: i32) -> ArrayRef {
    let float_array = Float32Array::from(values.to_vec());
    let field = Arc::new(Field::new("item", DataType::Float32, true));
    Arc::new(
        FixedSizeListArray::try_new(field, dim, Arc::new(float_array), None)
            .expect("Failed to create FixedSizeListArray"),
    )
}
