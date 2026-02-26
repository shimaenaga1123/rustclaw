use super::error::ToolError;
use rig::{completion::ToolDefinition, tool::Tool};
use rustypipe::client::RustyPipe;
use rustypipe::model::YouTubeItem;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Deserialize, Serialize)]
pub struct SearchYouTubeArgs {
    pub query: String,
    #[serde(default = "default_count")]
    pub count: i64,
}

fn default_count() -> i64 {
    5
}

#[derive(Clone)]
pub struct SearchYouTube {
    pub rp: Arc<RustyPipe>,
}

#[derive(Debug, Serialize)]
struct SearchVideoResult {
    video_id: String,
    title: String,
    channel: Option<String>,
    channel_id: Option<String>,
    duration: Option<String>,
    description: Option<String>,
    thumbnail: Option<String>,
    views: Option<u64>,
    length_seconds: Option<u32>,
    is_live: bool,
    is_short: bool,
}

impl Tool for SearchYouTube {
    const NAME: &'static str = "search_youtube";

    type Error = ToolError;
    type Args = SearchYouTubeArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Search YouTube videos and return metadata using rustypipe.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "YouTube search query"
                    },
                    "count": {
                        "type": "integer",
                        "description": "Number of results (1-50)",
                        "default": 5
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let count = args.count.clamp(1, 50) as usize;

        let search_results = self
            .rp
            .query()
            .search(&args.query)
            .await
            .map_err(|e| ToolError::SearchFailed(format!("YouTube search failed: {}", e)))?;

        let mut results = Vec::with_capacity(count);
        for item in &search_results.items.items {
            if let YouTubeItem::Video(video) = item {
                let thumbnail = video.thumbnail.first().map(|t| t.url.to_string());

                let duration = video.duration.unwrap_or_default();
                let duration_str = if duration > 0 {
                    let h = duration / 3600;
                    let m = (duration % 3600) / 60;
                    let s = duration % 60;
                    if h > 0 {
                        Some(format!("{}:{:02}:{:02}", h, m, s))
                    } else {
                        Some(format!("{}:{:02}", m, s))
                    }
                } else {
                    None
                };

                results.push(SearchVideoResult {
                    video_id: video.id.clone(),
                    title: normalize_ws(&video.name),
                    channel: video.channel.as_ref().map(|c| normalize_ws(&c.name)),
                    channel_id: video.channel.as_ref().map(|c| c.id.clone()),
                    duration: duration_str,
                    description: video.short_description.clone(),
                    thumbnail,
                    views: video.view_count,
                    length_seconds: video.duration,
                    is_live: video.is_live,
                    is_short: video.is_short,
                });
            }
            if results.len() >= count {
                break;
            }
        }

        let output = json!({
            "query": args.query,
            "count": results.len(),
            "results": results,
        });

        Ok(serde_json::to_string_pretty(&output).unwrap_or_default())
    }
}

fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
