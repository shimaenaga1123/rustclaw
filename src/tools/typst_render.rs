use super::error::ToolError;
use crate::agent::PendingFile;
use chrono::Datelike;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use typst::{
    Library, LibraryExt, World,
    diag::FileResult,
    foundations::{Bytes, Datetime},
    syntax::{FileId, Source},
    text::{Font, FontBook},
    utils::LazyHash,
};
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
pub struct TypstRenderArgs {
    pub content: String,
    #[serde(default)]
    pub filename: Option<String>,
}

#[derive(Clone)]
pub struct TypstRender {
    pub pending_files: Arc<RwLock<Vec<PendingFile>>>,
}

impl Tool for TypstRender {
    const NAME: &'static str = "typst_render";

    type Error = ToolError;
    type Args = TypstRenderArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Render Typst markup to a PNG image and send as a Discord attachment. \
                          Use for tables, math equations, formatted documents, and anything \
                          Discord markdown cannot render."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "Typst markup content to render"
                    },
                    "filename": {
                        "type": "string",
                        "description": "Optional filename (without extension). Defaults to 'render'."
                    }
                },
                "required": ["content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let content = args.content;
        let png_bytes = tokio::task::spawn_blocking(move || render_typst_to_png(&content))
            .await
            .map_err(|e| ToolError::CommandFailed(format!("Render task panicked: {}", e)))?
            .map_err(ToolError::CommandFailed)?;

        let filename = format!("{}.png", args.filename.as_deref().unwrap_or("render"));
        let tmp_path = std::env::temp_dir().join(format!(
            "typst_{}_{}",
            &Uuid::new_v4().to_string()[..8],
            &filename
        ));

        tokio::fs::write(&tmp_path, &png_bytes)
            .await
            .map_err(|e| ToolError::CommandFailed(format!("Failed to write PNG: {}", e)))?;

        let size = png_bytes.len();
        self.pending_files.write().await.push(PendingFile {
            filename: filename.clone(),
            path: tmp_path,
        });

        Ok(format!("Rendered '{}' ({} bytes)", filename, size))
    }
}

fn render_typst_to_png(content: &str) -> Result<Vec<u8>, String> {
    struct MiniWorld {
        library: LazyHash<Library>,
        book: LazyHash<FontBook>,
        fonts: Vec<Font>,
        source: Source,
    }

    impl MiniWorld {
        fn new(text: &str) -> Self {
            let mut book = FontBook::new();
            let mut fonts = Vec::new();
            for data in typst_assets::fonts() {
                let buffer = Bytes::new(data.to_vec());
                for font in Font::iter(buffer) {
                    book.push(font.info().clone());
                    fonts.push(font);
                }
            }
            Self {
                library: LazyHash::new(Library::default()),
                book: LazyHash::new(book),
                fonts,
                source: Source::detached(text),
            }
        }
    }

    impl World for MiniWorld {
        fn library(&self) -> &LazyHash<Library> {
            &self.library
        }
        fn book(&self) -> &LazyHash<FontBook> {
            &self.book
        }
        fn main(&self) -> FileId {
            self.source.id()
        }
        fn source(&self, id: FileId) -> FileResult<Source> {
            if id == self.source.id() {
                Ok(self.source.clone())
            } else {
                Err(typst::diag::FileError::NotFound(
                    id.vpath().as_rootless_path().into(),
                ))
            }
        }
        fn file(&self, id: FileId) -> FileResult<Bytes> {
            Err(typst::diag::FileError::NotFound(
                id.vpath().as_rootless_path().into(),
            ))
        }
        fn font(&self, index: usize) -> Option<Font> {
            self.fonts.get(index).cloned()
        }
        fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
            let now = chrono::Local::now();
            Datetime::from_ymd(now.year(), now.month() as u8, now.day() as u8)
        }
    }

    let world = MiniWorld::new(content);
    let document: typst::layout::PagedDocument = typst::compile(&world).output.map_err(|errs| {
        errs.into_iter()
            .map(|e| e.message.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })?;

    if document.pages.is_empty() {
        return Err("No pages generated".into());
    }

    let ppi = 3.0;
    let pixmaps: Vec<tiny_skia::Pixmap> = document
        .pages
        .iter()
        .map(|page| typst_render::render(page, ppi))
        .collect();

    if pixmaps.len() == 1 {
        return pixmaps[0].encode_png().map_err(|e| e.to_string());
    }

    let width = pixmaps.iter().map(|p| p.width()).max().unwrap();
    let total_height: u32 = pixmaps.iter().map(|p| p.height()).sum();
    let mut combined = tiny_skia::Pixmap::new(width, total_height)
        .ok_or_else(|| "Failed to create combined pixmap".to_string())?;

    let mut y = 0i32;
    for pm in &pixmaps {
        combined.draw_pixmap(
            0,
            y,
            pm.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            tiny_skia::Transform::identity(),
            None,
        );
        y += pm.height() as i32;
    }

    combined.encode_png().map_err(|e| e.to_string())
}
