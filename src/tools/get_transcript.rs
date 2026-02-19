use super::error::ToolError;
use rig::{completion::ToolDefinition, tool::Tool};
use rusty_ytdl::Video;
use serde::{Deserialize, Serialize};
use serde_json::json;
use yt_transcript_rs::YouTubeTranscriptApi;

#[derive(Deserialize, Serialize)]
pub struct GetTranscriptArgs {
    pub video_id_or_url: String,
    pub lang: Option<String>,
    #[serde(default = "default_max_chars")]
    pub max_chars: i64,
    #[serde(default)]
    pub include_timestamps: bool,
}

fn default_max_chars() -> i64 {
    20000
}

#[derive(Clone, Default)]
pub struct GetTranscript {}

#[derive(Debug, Serialize)]
struct TranscriptSegment {
    start_seconds: f64,
    duration_seconds: f64,
    text: String,
}

impl Tool for GetTranscript {
    const NAME: &'static str = "get_transcript";

    type Error = ToolError;
    type Args = GetTranscriptArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description:
                "Fetch a YouTube transcript. Uses rusty_ytdl for video validation/metadata and yt-transcript-rs for caption retrieval."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "video_id_or_url": {
                        "type": "string",
                        "description": "YouTube video ID or URL"
                    },
                    "lang": {
                        "type": "string",
                        "description": "Preferred transcript language code (e.g. en, ko, ja)"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Maximum transcript characters to return (1000-200000)",
                        "default": 20000
                    },
                    "include_timestamps": {
                        "type": "boolean",
                        "description": "Include per-segment timestamps",
                        "default": false
                    }
                },
                "required": ["video_id_or_url"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let video = Video::new(args.video_id_or_url.as_str())
            .map_err(|e| ToolError::SearchFailed(format!("Invalid YouTube video id/url: {}", e)))?;
        let video_id = video.get_video_id();
        let video_title = video
            .get_basic_info()
            .await
            .map_err(|e| ToolError::SearchFailed(format!("Failed to fetch video info: {}", e)))?
            .video_details
            .title;

        let api = YouTubeTranscriptApi::new(None, None, None).map_err(|e| {
            ToolError::SearchFailed(format!("Failed to initialize yt-transcript-rs API: {}", e))
        })?;
        let preferred_languages = preferred_languages(args.lang.as_deref());
        let preferred_refs: Vec<&str> = preferred_languages.iter().map(String::as_str).collect();
        let transcript = api
            .fetch_transcript(&video_id, &preferred_refs, false)
            .await
            .map_err(|e| ToolError::SearchFailed(format!("Failed to fetch transcript: {}", e)))?;

        let mut segments = transcript
            .parts()
            .iter()
            .filter_map(|part| {
                let text = normalize_ws(part.text.as_str());
                if text.is_empty() {
                    return None;
                }
                Some(TranscriptSegment {
                    start_seconds: round_ms(part.start),
                    duration_seconds: round_ms(part.duration),
                    text,
                })
            })
            .collect::<Vec<_>>();

        if segments.is_empty() {
            let fallback_text = normalize_ws(&transcript.text());
            if !fallback_text.is_empty() {
                segments.push(TranscriptSegment {
                    start_seconds: 0.0,
                    duration_seconds: 0.0,
                    text: fallback_text,
                });
            }
        }
        if segments.is_empty() {
            return Err(ToolError::SearchFailed(format!(
                "Transcript data exists but no text segments were found for {}",
                video_id
            )));
        }

        let full_text = normalize_ws(&transcript.text());

        let max_chars = args.max_chars.clamp(1000, 200000) as usize;
        let full_char_count = full_text.chars().count();
        let truncated = full_char_count > max_chars;
        let text = if truncated {
            full_text.chars().take(max_chars).collect::<String>()
        } else {
            full_text
        };

        let mut output = json!({
            "video_id": video_id,
            "title": video_title,
            "language": transcript.language_code(),
            "language_name": transcript.language(),
            "is_auto_generated": transcript.is_generated(),
            "segment_count": segments.len(),
            "char_count": text.chars().count(),
            "original_char_count": full_char_count,
            "truncated": truncated,
            "text": text,
        });

        if args.include_timestamps {
            output["segments"] = serde_json::to_value(&segments).unwrap_or_else(|_| json!([]));
        }

        Ok(serde_json::to_string_pretty(&output).unwrap_or_default())
    }
}

fn preferred_languages(lang: Option<&str>) -> Vec<String> {
    let mut list = Vec::new();
    if let Some(lang) = lang {
        let lang = normalize_lang_code(lang);
        if !lang.is_empty() {
            list.push(lang);
        }
    }
    for fallback in ["ko", "en", "ja", "zh", "de", "fr", "es", "pt"] {
        if !list.iter().any(|v| v == fallback) {
            list.push(fallback.to_string());
        }
    }
    list
}

fn normalize_lang_code(code: &str) -> String {
    code.trim().to_lowercase()
}

fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn round_ms(value_ms: f64) -> f64 {
    let seconds = value_ms / 1000.0;
    (seconds * 1000.0).round() / 1000.0
}
