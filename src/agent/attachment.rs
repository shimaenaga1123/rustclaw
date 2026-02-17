use std::fmt::Write;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct PendingFile {
    pub filename: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    pub filename: String,
    pub container_path: String,
    pub size: u32,
    pub content_type: Option<String>,
}

impl AttachmentInfo {
    pub fn format_for_prompt(attachments: &[AttachmentInfo]) -> String {
        if attachments.is_empty() {
            return String::new();
        }

        let mut out = String::from("[Attachments uploaded to /workspace/upload/]\n");
        for att in attachments {
            let _ = writeln!(
                out,
                "- {} ({} bytes, {}): {}",
                att.filename,
                att.size,
                att.content_type.as_deref().unwrap_or("unknown"),
                att.container_path,
            );
        }
        out
    }
}
