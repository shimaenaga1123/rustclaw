use serenity::model::channel::ReactionType;
use std::path::Path;

use super::CANCEL_EMOJI;

pub(super) fn cancel_emoji() -> ReactionType {
    ReactionType::Unicode(CANCEL_EMOJI.to_string())
}

pub(super) fn sanitize_filename(name: &str) -> String {
    name.replace(['/', '\\', '\0', ':', '*', '?', '"', '<', '>', '|'], "_")
        .trim()
        .chars()
        .take(200)
        .collect()
}

pub(super) fn deduplicate_filename(dir: &Path, filename: &str) -> String {
    if !dir.join(filename).exists() {
        return filename.to_string();
    }

    let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let (stem, ext) = match filename.rfind('.') {
        Some(i) => (&filename[..i], &filename[i..]),
        None => (filename, ""),
    };
    format!("{}_{}{}", stem, ts, ext)
}

fn char_count_to_byte_pos(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

pub(super) fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.chars().count() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;
    let mut open_code_block: Option<String> = None;

    while !remaining.is_empty() {
        let prefix = match &open_code_block {
            Some(lang) => format!("```{lang}\n"),
            None => String::new(),
        };
        let suffix_reserve = if open_code_block.is_some() { 4 } else { 0 };
        let available = max_len - prefix.chars().count() - suffix_reserve;

        if remaining.chars().count() <= available {
            let mut chunk = prefix;
            chunk.push_str(remaining);
            if !chunk.trim().is_empty() {
                chunks.push(chunk);
            }
            break;
        }

        let boundary = char_count_to_byte_pos(remaining, available);
        let split_at = remaining[..boundary]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or_else(|| {
                remaining[..boundary]
                    .rfind(' ')
                    .map(|i| i + 1)
                    .unwrap_or(boundary)
            });

        let slice = &remaining[..split_at];

        let mut chunk = prefix;
        chunk.push_str(slice);

        update_code_block_state(&mut open_code_block, slice);

        if open_code_block.is_some() {
            chunk.push_str("\n```");
        }

        if !chunk.trim().is_empty() {
            chunks.push(chunk);
        }
        remaining = &remaining[split_at..];
    }

    if chunks.is_empty() {
        chunks.push(text.chars().take(max_len).collect());
    }

    chunks
}

fn update_code_block_state(state: &mut Option<String>, text: &str) {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("```") {
            if state.is_some() {
                *state = None;
            } else {
                *state = Some(rest.trim().to_string());
            }
        }
    }
}

pub(super) fn split_streaming(accumulated: &str, max_len: usize) -> (String, String) {
    let boundary = char_count_to_byte_pos(accumulated, max_len);
    let split_at = accumulated[..boundary]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or_else(|| {
            accumulated[..boundary]
                .rfind(' ')
                .map(|i| i + 1)
                .unwrap_or(boundary)
        });

    let chunk_text = &accumulated[..split_at];
    let rest = &accumulated[split_at..];

    let mut code_block_state: Option<String> = None;
    update_code_block_state(&mut code_block_state, chunk_text);

    if let Some(ref lang) = code_block_state {
        let send = format!("{chunk_text}\n```");
        let carry = format!("```{lang}\n{rest}");
        (send, carry)
    } else {
        (chunk_text.to_string(), rest.to_string())
    }
}
