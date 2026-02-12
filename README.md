# RustClaw

A lightweight, memory-aware Discord AI assistant powered by multi-provider LLM APIs, implemented in Rust.

Inspired by [NanoClaw](https://github.com/gavrielc/nanoclaw) and [OpenClaw](https://github.com/openclaw/openclaw).

## Features

- **Discord Integration** — Mention-based interaction with streaming responses
- **Multi-Provider LLM** — Anthropic, OpenAI, Gemini via [Rig](https://github.com/0xPlaygrounds/rig)
- **Vector Memory** — usearch (F16 HNSW) + SQLite semantic memory with recent turns + similarity search
- **Pluggable Embeddings** — Local (fastembed, 384d) or Gemini API (768d, near-zero RAM)
- **Sandboxed Execution** — All commands run in isolated Debian Docker containers (Bun pre-installed)
- **Tool Calling** — Shell commands, web search, weather, Typst rendering, file sending, cron scheduler
- **Owner/User Permissions** — AI-aware permission system for safe multi-user operation
- **Auto-Update** — Daily binary updates via cargo-dist (systemd/launchd)

## Quick Start

### Install

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/shimaenaga1123/rustclaw/releases/latest/download/rustclaw-installer.sh | sh
```

### Setup Service (optional)

```bash
curl -fsSL https://raw.githubusercontent.com/shimaenaga1123/rustclaw/main/setup-service.sh | bash
```

### Build from Source

```bash
git clone https://github.com/shimaenaga1123/rustclaw && cd rustclaw
cp config.example.toml config.toml  # edit with your credentials
cargo run --release
```

### Prerequisites

- Discord Bot Token (with Message Content Intent enabled)
- Anthropic-compatible API Key
- Docker running locally
- Brave Search API Key (optional)
- Gemini API Key (optional, for embedding provider)

## Configuration

```toml
[discord]
token = "your_discord_bot_token"
owner_id = 123456789012345678

[api]
provider = "anthropic"  # or "openai", "gemini"
key = "your_api_key"
url = "https://api.anthropic.com/v1"
model = "claude-sonnet-4-20250514"

[brave]
# api_key = "optional_brave_key"

[storage]
data_dir = "data"

[commands]
timeout = 30

[model]
disable_reasoning = false

[embedding]
provider = "local"  # or "gemini"
# api_key = "gemini_key"    # omit to reuse [api].key
# model = "gemini-embedding-001"
# dimensions = 768
```

> **Switching embedding providers**: Delete `data/conversations.usearch` when changing between local (384d) and gemini (768d) — dimensions are incompatible.

## Tools

| Tool | Description | Permission |
|------|-------------|------------|
| `run_command` | Shell commands in Docker sandbox | All |
| `send_file` | Send workspace files as Discord attachments | All |
| `typst_render` | Render Typst markup (tables, math) to PNG | All |
| `web_search` | Brave Search web lookup | All |
| `weather` | Current weather and forecast | All |
| `search_memory` | Semantic search over past conversations | All |
| `schedule` / `list_schedules` | Create and list cron tasks | All |
| `important_add` / `important_list` / `important_delete` | Manage persistent key facts | Owner only (add/delete) |
| `unschedule` | Remove a scheduled task | Owner only |
| `reset_container` | Reset Docker sandbox | Owner only |

## Memory System

```
┌─────────────────────────────────┐
│  # Important Facts              │  ← All entries, always included
├─────────────────────────────────┤
│  # Recent Conversations         │  ← Last 5 turns (chronological)
├─────────────────────────────────┤
│  # Related Past Conversations   │  ← Top 10 semantic matches
└─────────────────────────────────┘
```

- **Storage**: SQLite (`data/memory.db`) + usearch index (`data/conversations.usearch`)
- **Embedding model cache**: `data/models/` (local provider only, ~130MB)

## Data Layout

```
data/
├── memory.db              # SQLite (conversations + important facts)
├── conversations.usearch  # Vector index (F16 quantized)
├── models/                # Embedding model cache (local only)
├── workspace/             # Docker sandbox mount
└── schedules.json         # Persisted cron tasks
```

## License

MIT — see [LICENSE](LICENSE)

## Acknowledgments

[Rig](https://github.com/0xPlaygrounds/rig) · [serenity](https://github.com/serenity-rs/serenity) · [usearch](https://github.com/unum-cloud/usearch) · [fastembed](https://github.com/Anush008/fastembed-rs) · [Typst](https://typst.app/) · [cargo-dist](https://github.com/axodotdev/cargo-dist)
