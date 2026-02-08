use crate::vectordb::VectorDb;
use anyhow::Result;
use std::sync::Arc;
use tracing::info;

const RECENT_TURN_COUNT: usize = 20;
const SEMANTIC_SEARCH_COUNT: usize = 10;

pub struct MemoryManager {
    vectordb: Arc<VectorDb>,
}

impl MemoryManager {
    pub async fn new(vectordb: Arc<VectorDb>) -> Result<Arc<Self>> {
        info!("MemoryManager initialized (LanceDB backend)");
        Ok(Arc::new(Self { vectordb }))
    }

    pub async fn add_turn(
        &self,
        author: &str,
        user_input: &str,
        assistant_response: &str,
    ) -> Result<()> {
        self.vectordb
            .add_turn(author, user_input, assistant_response)
            .await
    }

    pub async fn get_context(&self, current_input: &str) -> Result<String> {
        let mut context = String::new();

        let important = self.vectordb.get_important_context().await?;
        if !important.is_empty() {
            context.push_str(&important);
            context.push('\n');
        }

        let recent = self.vectordb.recent_turns(RECENT_TURN_COUNT).await?;
        let recent_ids: Vec<String> = recent.iter().map(|t| t.id.clone()).collect();

        if !recent.is_empty() {
            context.push_str("# Recent Conversations\n\n");
            for turn in &recent {
                context.push_str(&turn.format_for_context());
                context.push_str("\n\n");
            }
        }

        let semantic = self
            .vectordb
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

    pub fn vectordb(&self) -> &Arc<VectorDb> {
        &self.vectordb
    }
}
