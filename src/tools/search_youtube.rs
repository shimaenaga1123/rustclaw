use super::error::ToolError;
use rig::{completion::ToolDefinition, tool::Tool};
use rusty_ytdl::search::{SearchOptions, SearchResult, SearchType, YouTube};
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
    pub yt: Arc<YouTube>,
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
            description: "Search YouTube videos and return metadata.".to_string(),
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

        let options = SearchOptions {
            limit: count as u64,
            search_type: SearchType::Video,
            safe_search: false,
        };

        let search_results = self
            .yt
            .search(&args.query, Some(&options))
            .await
            .map_err(|e| ToolError::SearchFailed(format!("YouTube search failed: {}", e)))?;

        let mut results = Vec::with_capacity(count);
        for item in &search_results {
            if let SearchResult::Video(video) = item {
                let thumbnail = video.thumbnails.first().map(|t| t.url.clone());

                let duration_secs = video.duration;
                let duration_str = if duration_secs > 0 {
                    let h = duration_secs / 3600;
                    let m = (duration_secs % 3600) / 60;
                    let s = duration_secs % 60;
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
                    title: normalize_ws(&video.title),
                    channel: Some(normalize_ws(&video.channel.name)),
                    channel_id: Some(video.channel.id.clone()),
                    duration: duration_str,
                    description: if video.description.is_empty() {
                        None
                    } else {
                        Some(video.description.clone())
                    },
                    thumbnail,
                    views: Some(video.views),
                    length_seconds: if duration_secs > 0 {
                        Some(duration_secs as u32)
                    } else {
                        None
                    },
                    is_live: false,
                    is_short: false,
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
