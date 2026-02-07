pub fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        let split_at = remaining[..max_len]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or_else(|| {
                remaining[..max_len]
                    .rfind(' ')
                    .map(|i| i + 1)
                    .unwrap_or(max_len)
            });

        let chunk = &remaining[..split_at];
        if !chunk.trim().is_empty() {
            chunks.push(chunk.to_string());
        }
        remaining = &remaining[split_at..];
    }

    if chunks.is_empty() {
        chunks.push(text.chars().take(max_len).collect());
    }

    chunks
}
