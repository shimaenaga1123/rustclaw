use super::error::ToolError;
use rig::{completion::ToolDefinition, tool::Tool};
use rusty_ytdl::search::{SearchOptions, SearchResult, SearchType, YouTube};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize, Serialize)]
pub struct SearchYouTubeArgs {
    pub query: String,
    #[serde(default = "default_count")]
    pub count: i64,
}

fn default_count() -> i64 {
    5
}

#[derive(Clone, Default)]
pub struct SearchYouTube {}

#[derive(Debug, Serialize)]
struct SearchVideoResult {
    video_id: String,
    title: String,
    channel: Option<String>,
    published_at: Option<String>,
    duration: Option<String>,
    description: Option<String>,
    thumbnail: Option<String>,
    views: Option<u64>,
    length_seconds: Option<u64>,
}

impl Tool for SearchYouTube {
    const NAME: &'static str = "search_youtube";

    type Error = ToolError;
    type Args = SearchYouTubeArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Search YouTube videos and return metadata using rusty_ytdl.".to_string(),
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
        let count = args.count.clamp(1, 50) as u64;
        let youtube = YouTube::new().map_err(|e| {
            ToolError::SearchFailed(format!("Failed to initialize search client: {}", e))
        })?;
        let options = SearchOptions {
            limit: count,
            search_type: SearchType::Video,
            safe_search: false,
        };
        let search_results = youtube
            .search(args.query.clone(), Some(&options))
            .await
            .map_err(|e| ToolError::SearchFailed(format!("YouTube search failed: {}", e)))?;

        let mut results = Vec::with_capacity(count as usize);
        for entry in search_results {
            if let SearchResult::Video(video) = entry {
                let thumbnail = video
                    .thumbnails
                    .iter()
                    .max_by_key(|thumb| thumb.width * thumb.height)
                    .map(|thumb| thumb.url.clone());
                results.push(SearchVideoResult {
                    video_id: video.id,
                    title: normalize_ws(&video.title),
                    channel: Some(normalize_ws(&video.channel.name)),
                    published_at: video.uploaded_at,
                    duration: Some(video.duration_raw),
                    description: truncate_chars(&normalize_ws(&video.description), 300),
                    thumbnail,
                    views: Some(video.views),
                    length_seconds: Some(video.duration),
                });
            }
            if results.len() >= count as usize {
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

fn truncate_chars(text: &str, max_chars: usize) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    if text.chars().count() > max_chars {
        let truncated = text.chars().take(max_chars).collect::<String>();
        Some(format!("{}...", truncated))
    } else {
        Some(text.to_string())
    }
}
