use crate::vector_db::VectorDb;
use anyhow::Result;
use std::sync::Arc;
use tracing::info;

const RECENT_TURN_COUNT: usize = 3;
const SEMANTIC_SEARCH_COUNT: usize = 5;

pub struct MemoryManager {
    vector_db: Arc<VectorDb>,
}

impl MemoryManager {
    pub async fn new(vector_db: Arc<VectorDb>) -> Result<Arc<Self>> {
        info!("MemoryManager initialized (usearch + SQLite backend)");
        Ok(Arc::new(Self { vector_db }))
    }

    pub async fn add_turn(
        &self,
        author: &str,
        user_input: &str,
        assistant_response: &str,
    ) -> Result<()> {
        self.vector_db
            .add_turn(author, user_input, assistant_response)
            .await
    }

    pub async fn get_context(&self, current_input: &str) -> Result<String> {
        let mut context = String::new();

        let important = self.vector_db.get_important_context().await?;
        if !important.is_empty() {
            context.push_str(&important);
            context.push('\n');
        }

        let recent = self.vector_db.recent_turns(RECENT_TURN_COUNT).await?;
        let recent_ids: Vec<String> = recent.iter().map(|t| t.id.clone()).collect();

        if !recent.is_empty() {
            context.push_str("# Recent Conversations\n\n");
            for turn in &recent {
                context.push_str(&turn.format_for_context());
                context.push_str("\n\n");
            }
        }

        let semantic = self
            .vector_db
            .search_turns(current_input, SEMANTIC_SEARCH_COUNT, &recent_ids)
            .await?;

        if !semantic.is_empty() {
            context.push_str("# Related Past Conversations\n\n");
            for turn in &semantic {
                context.push_str(&turn.format_for_context());
                context.push_str("\n\n");
            }
        }

        Ok(context)
    }

    pub fn vector_db(&self) -> &Arc<VectorDb> {
        &self.vector_db
    }
}
