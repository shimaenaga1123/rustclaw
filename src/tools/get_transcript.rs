use super::error::ToolError;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use rig::{completion::ToolDefinition, tool::Tool};
use rustypipe::client::RustyPipe;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

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

#[derive(Clone)]
pub struct GetTranscript {
    pub rp: Arc<RustyPipe>,
    pub client: reqwest::Client,
}

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
            "Fetch a YouTube transcript. Uses rustypipe for video metadata and subtitle retrieval."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "video_id_or_url": {
                        "type": "string",
                        "description": "YouTube video ID or youtu.be URL"
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
        let video_id = extract_video_id(&args.video_id_or_url);

        let details = self
            .rp
            .query()
            .video_details(&video_id)
            .await
            .map_err(|e| {
                ToolError::SearchFailed(format!("Failed to fetch video details: {}", e))
            })?;
        let video_title = details.name.clone();

        let player =
            self.rp.query().player(&video_id).await.map_err(|e| {
                ToolError::SearchFailed(format!("Failed to fetch player data: {}", e))
            })?;

        if player.subtitles.is_empty() {
            return Err(ToolError::SearchFailed(format!(
                "No subtitles available for video {}",
                video_id
            )));
        }

        let preferred = preferred_languages(args.lang.as_deref());
        let (track, is_auto) =
            select_subtitle_track(&player.subtitles, &preferred).ok_or_else(|| {
                ToolError::SearchFailed(format!(
                    "No matching subtitle track found for video {}",
                    video_id
                ))
            })?;

        let body = self
            .client
            .get(track.url.as_str())
            .send()
            .await
            .map_err(|e| ToolError::SearchFailed(format!("Failed to fetch subtitle data: {}", e)))?
            .text()
            .await
            .map_err(|e| {
                ToolError::SearchFailed(format!("Failed to read subtitle response: {}", e))
            })?;

        let segments = parse_subtitle_xml(&body)?;

        if segments.is_empty() {
            return Err(ToolError::SearchFailed(format!(
                "Transcript data exists but no text segments were found for {}",
                video_id
            )));
        }

        let full_text: String = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let full_text = normalize_ws(&full_text);

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
            "language": track.lang.to_string(),
            "is_auto_generated": is_auto,
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

fn extract_video_id(input: &str) -> String {
    let input = input.trim();
    input
        .strip_prefix("clients://youtu.be/")
        .or_else(|| input.strip_prefix("client://youtu.be/"))
        .map(|rest| rest.split(['?', '&', '/']).next().unwrap_or(rest))
        .unwrap_or(input)
        .to_string()
}

fn select_subtitle_track<'a>(
    subtitles: &'a [rustypipe::model::Subtitle],
    preferred: &[String],
) -> Option<(&'a rustypipe::model::Subtitle, bool)> {
    for lang in preferred {
        for sub in subtitles {
            if sub.lang.as_str().starts_with(lang.as_str()) && !sub.auto_generated {
                return Some((sub, false));
            }
        }
    }
    for lang in preferred {
        for sub in subtitles {
            if sub.lang.as_str().starts_with(lang.as_str()) && sub.auto_generated {
                return Some((sub, true));
            }
        }
    }
    subtitles.first().map(|s| (s, s.auto_generated))
}

fn parse_subtitle_xml(xml: &str) -> Result<Vec<TranscriptSegment>, ToolError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut segments = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let tag = e.local_name();
                let (start_attr, dur_attr, is_ms) = match tag.as_ref() {
                    b"text" => (b"start".as_slice(), b"dur".as_slice(), false),
                    b"p" => (b"t".as_slice(), b"d".as_slice(), true),
                    _ => {
                        buf.clear();
                        continue;
                    }
                };

                let mut start: f64 = 0.0;
                let mut dur: f64 = 0.0;
                for attr in e.attributes().flatten() {
                    if attr.key.as_ref() == start_attr {
                        start = std::str::from_utf8(&attr.value)
                            .unwrap_or("0")
                            .parse()
                            .unwrap_or(0.0);
                    } else if attr.key.as_ref() == dur_attr {
                        dur = std::str::from_utf8(&attr.value)
                            .unwrap_or("0")
                            .parse()
                            .unwrap_or(0.0);
                    }
                }

                if is_ms {
                    start /= 1000.0;
                    dur /= 1000.0;
                }

                // read_text consumes until the matching end tag, returning inner content
                let inner = reader.read_text(e.name()).unwrap_or_default().into_owned();
                let stripped = strip_xml_tags(&inner);
                let text = normalize_ws(&stripped);

                if !text.is_empty() {
                    segments.push(TranscriptSegment {
                        start_seconds: round3(start),
                        duration_seconds: round3(dur),
                        text,
                    });
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(ToolError::SearchFailed(format!(
                    "Failed to parse subtitle XML: {}",
                    e
                )));
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(segments)
}

fn strip_xml_tags(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut inside_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

fn preferred_languages(lang: Option<&str>) -> Vec<String> {
    let mut list = Vec::new();
    if let Some(lang) = lang {
        let lang = lang.trim().to_lowercase();
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

fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}
