# RustClaw

A lightweight, memory-aware Discord AI assistant powered by Anthropic-compatible APIs, implemented in Rust. Inspired by [nanoclaw](https://github.com/gavrielc/nanoclaw) and [OpenClaw](https://github.com/openclaw/openclaw)

## Features

- **Discord Integration**: Slash command (`/ask`) interaction
- **Anthropic-compatible API**: Works with Claude, Minimax, and other Anthropic-compatible endpoints via [Rig](https://github.com/0xPlaygrounds/rig)
- **Pluggable Embedding System**: Choose between local or API-based embeddings
  - **Local**: fastembed (multilingual-e5-small, 384d) — no API dependency, GPU-free
  - **Gemini API**: Google's gemini-embedding-001 (768d, configurable) — near-zero RAM usage
- **Vector Memory System**: usearch (F16 quantized) + SQLite powered semantic memory
  - **Important Facts**: Owner-managed persistent facts, always included in context
  - **Conversation History**: Every turn stored with embeddings for semantic retrieval
  - **Hybrid Context**: Recent 5 turns + top 10 semantically similar past conversations
  - **On-demand Search**: Explicit semantic search tool for deeper memory recall
- **Tool Calling**: Shell command execution, web search, memory operations, and Typst rendering
- **Typst Rendering**: Tables, math equations, and formatted content rendered as PNG via embedded Typst compiler
- **Sandboxed Execution**: All commands run in isolated Debian Docker containers with Bun runtime
- **Owner Permission System**: Owner/non-owner distinction with AI-level awareness for safe multi-user operation
- **Brave Search**: Optional web search integration
- **Task Scheduler**: Cron-based task scheduling with persistence
- **Auto-Update**: Automatic version checking and updating (systemd on Linux, launchd on macOS)

## Prerequisites

- Rust 1.70+ (for building from source)
- Discord Bot Token
- Anthropic-compatible API Key (Claude, Minimax, etc.)
- Docker (required for all command execution)
- Brave Search API Key (optional)
- Google Gemini API Key (optional, for Gemini embedding provider)

## Quick Start

### Install Binary (Recommended)

Supports **Linux** (x86_64) and **macOS** (Apple Silicon).
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/shimaenaga1123/rustclaw/releases/latest/download/rustclaw-installer.sh | sh
```

### Setup Background Service

After installing the binary, set up a background service and auto-updater:
```bash
curl -fsSL https://raw.githubusercontent.com/shimaenaga1123/rustclaw/main/setup-service.sh | bash
```

Then edit the config and start:
```bash
nano ~/.local/share/rustclaw/config.toml

# Linux
systemctl --user start rustclaw

# macOS
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.rustclaw.bot.plist
```

### Build from Source
```bash
git clone https://github.com/shimaenaga1123/rustclaw
cd rustclaw
cp config.example.toml config.toml
# Edit config.toml with your credentials
cargo run --release
```

On first run with the local embedding provider, the embedding model (~130MB) will be downloaded from HuggingFace automatically.

### Create Discord Bot

1. Go to [Discord Developer Portal](https://discord.com/developers/applications)
2. Create New Application → Bot
3. Enable "Message Content Intent"
4. Copy token to `config.toml`
5. Invite bot with permissions: `View Channels`, `Send Messages`, `Read Message History`

### Prepare Docker

Ensure Docker is running. The bot will automatically pull the `oven/bun:debian` image on first command execution.

```bash
docker pull oven/bun:debian  # optional, speeds up first run
```

## Auto-Update

RustClaw automatically checks for new releases daily and updates the binary.

### How It Works

- **Linux**: `rustclaw-update.timer` runs daily at 04:00 (±30 min randomized delay)
- **macOS**: `com.rustclaw.update` launchd agent runs daily at 04:00
- Uses cargo-dist's built-in updater (`rustclaw-update`) for downloading and verification
- After updating, the main service is automatically restarted

### Check Installed Version

```bash
cat ~/.local/share/rustclaw/version
```

## Usage

Use the `/ask` slash command to interact with the bot:

```
/ask prompt:What's the weather like today?

/ask prompt:search for rust async programming

/ask prompt:run ls -la

/ask prompt:remember that I prefer dark mode
```

You can also attach files using the optional `file` parameter.

## Permission System

RustClaw distinguishes between the **bot owner** and **regular users**. The AI model is aware of the caller's permission level and adjusts its behavior accordingly.

### Owner

- Full administrative privileges
- Can create, list, and **remove** scheduled tasks
- Can **add**, **list**, and **delete** important memory entries
- Can **reset** the Docker container
- No output truncation

### Regular Users

- Output truncated to 4096 characters
- Can create and list scheduled tasks, but **cannot remove** them
- Can view important facts in context, but **cannot modify** them
- AI refuses requests that could affect the host system, reveal internal configuration, or escalate privileges

## Embedding Providers

RustClaw supports pluggable embedding backends via the `[embedding]` config section.

### Local (default)

Uses [fastembed](https://github.com/Anush008/fastembed-rs) with the `multilingual-e5-small` model for fully offline, GPU-free embedding.

- **Dimensions**: 384
- **RAM usage**: ~300–500MB (ONNX Runtime, auto-unloads after 5 min idle)
- **Latency**: ~10–50ms per embedding
- **No API key required**

```toml
[embedding]
provider = "local"
```

### Gemini API

Uses Google's [Gemini Embedding API](https://ai.google.dev/gemini-api/docs/embeddings) for near-zero memory overhead.

- **Dimensions**: 768 (configurable down to 64 via Matryoshka)
- **RAM usage**: near-zero (HTTP calls only)
- **Free tier**: generous limits, sufficient for most Discord bots
- **Requires**: Gemini API key ([get one free](https://aistudio.google.com/apikey))

```toml
[embedding]
provider = "gemini"
# api_key = "your_gemini_key"  # omit to reuse [api].key
# model = "gemini-embedding-001"
# dimensions = 768
```

### Switching Providers

When switching between providers, the embedding dimensions change (384 vs 768), so the existing vector index is incompatible. Delete the old index before restarting:

```bash
rm data/conversations.usearch
```

Conversation text in SQLite (`data/memory.db`) is preserved — only the vector index is rebuilt as new conversations come in. Existing conversations will not be searchable until re-embedded.

## Memory System

RustClaw uses a vector-based memory system powered by **usearch** (HNSW index, F16 quantized) and **SQLite** for storage.

### Important Facts (`important` table)
- Owner-managed key facts (preferences, dates, decisions)
- Always loaded in full into every conversation context
- CRUD via `important_add`, `important_list`, `important_delete` tools (owner only)

### Conversation History (`conversations` table)
- Every conversation turn (user input + assistant response) stored immediately
- Each turn is embedded and indexed in usearch for semantic retrieval
- No compression or summarization — original text preserved
- Context includes:
  - **Recent 5 turns**: maintains conversation flow and continuity
  - **Semantic top 10**: past turns most relevant to the current input (deduplicated against recent)

### On-demand Search (`search_memory` tool)
- Explicitly search past conversations by semantic similarity
- Returns up to 20 matching turns with timestamps and content
- Useful when the automatic context window doesn't cover the needed history

### How Context is Built
```
┌─────────────────────────────────┐
│  # Important Facts              │  ← All entries, always
│  - User prefers dark mode       │
│  - Birthday is January 1st      │
├─────────────────────────────────┤
│  # Recent Conversations         │  ← Last 5 turns (chronological)
│  User: ...                      │
│  Assistant: ...                 │
├─────────────────────────────────┤
│  # Related Past Conversations   │  ← Top 10 semantic matches
│  User: ...                      │     (excluding recent turns)
│  Assistant: ...                 │
└─────────────────────────────────┘
```

### Data Storage

- Metadata and text: `data/memory.db` (SQLite)
- Vector index: `data/conversations.usearch` (usearch HNSW, F16 quantized)
- Embedding model cache: `data/models/` (local provider only)

## Tools

The bot has access to these tools:

### `run_command`
Execute shell commands in an isolated Debian Docker container with Bun runtime:
```
/ask prompt:run bun --version
```

- All users' commands run inside `oven/bun:debian` containers
- Bun and Node.js are pre-installed
- Use `apt-get install` to install additional packages within the container

### `reset_container`
Reset the Docker sandbox container (owner only):
```
/ask prompt:reset the container
```

- Stops and removes the current container
- Clears the workspace directory
- A fresh container is created on the next command

### `web_search`
Search the web using Brave API:
```
/ask prompt:search for latest rust news
```

### `typst_render`
Render Typst markup as a PNG image and send as a Discord attachment:
```
/ask prompt:render this table: | Name | Score | | Alice | 95 |

/ask prompt:render the equation x^2 + y^2 = z^2
```

- Tables, math equations, and formatted documents
- Rendered via the embedded Typst compiler (no external binary needed)
- Fonts bundled via typst-assets

### `search_memory`
Search past conversations semantically:
```
/ask prompt:search memory for our discussion about database migration
```

- Returns the most relevant past conversation turns
- Useful for recalling specific past discussions

### `important_add`
Save important information to persistent memory (owner only):
```
/ask prompt:remember my birthday is January 1st
```

### `important_list`
List all stored important facts:
```
/ask prompt:show all important facts
```

### `important_delete`
Remove an important entry by ID (owner only):
```
/ask prompt:delete important entry abc123
```

### `weather`
Get current weather and forecast for any location:
```
/ask prompt:what's the weather in Seoul?
```

### `send_file`
Send files from the Docker workspace as Discord attachments:
```
/ask prompt:create a script and send it to me
```

### `schedule`
Schedule recurring tasks with cron expressions:
```
/ask prompt:schedule a daily weather check at 9am
```

### `list_schedules`
List all scheduled tasks:
```
/ask prompt:show my scheduled tasks
```

### `unschedule`
Remove a scheduled task (owner only):
```
/ask prompt:remove schedule abc123
```

Non-owner users will receive a permission denied error.

## Scheduler

The scheduler allows you to set up recurring tasks using cron expressions.

### Cron Expression Format

```
┌───────────── second (0-59)
│ ┌───────────── minute (0-59)
│ │ ┌───────────── hour (0-23)
│ │ │ ┌───────────── day of month (1-31)
│ │ │ │ ┌───────────── month (1-12)
│ │ │ │ │ ┌───────────── day of week (0-6, Sun-Sat)
│ │ │ │ │ │
* * * * * *
```

### Examples

- `0 0 9 * * *` - Daily at 9:00 AM
- `0 30 8 * * 1-5` - Weekdays at 8:30 AM
- `0 0 */2 * * *` - Every 2 hours
- `0 0 0 1 * *` - First day of each month at midnight

### Persistence

Scheduled tasks are automatically saved to `data/schedules.json` and restored on restart.

## Project Structure

```
rustclaw/
├── src/
│   ├── main.rs           # Entry point, embedding provider selection, graceful shutdown
│   ├── config.rs         # Configuration (including [embedding] section)
│   ├── utils.rs          # Shared utilities (split_message, etc.)
│   ├── embeddings.rs     # EmbeddingService trait + LocalEmbedding / GeminiEmbedding
│   ├── vectordb.rs       # usearch (F16) + SQLite wrapper (conversations + important)
│   ├── agent.rs          # AI agent + Rig integration
│   ├── tools/            # Tool implementations
│   ├── discord.rs        # Discord event handler (/ask slash command)
│   ├── memory.rs         # Memory manager (delegates to VectorDb)
│   └── scheduler.rs      # Task scheduler
└── data/
    ├── memory.db         # SQLite database (conversations + important)
    ├── conversations.usearch  # usearch vector index (F16 quantized)
    ├── models/           # Cached embedding model (~130MB, local provider only)
    ├── workspace/        # Docker sandbox mount
    └── schedules.json    # Scheduled tasks
```

## Configuration

See `config.example.toml` for all available options.

### Minimal Config

```toml
[discord]
token = "your_discord_bot_token"
owner_id = 123456789012345678

[api]
provider = "anthropic"
key = "your_api_key"
url = "https://api.anthropic.com/v1"
model = "claude-sonnet-4-20250514"

[brave]

[storage]
data_dir = "data"

[commands]
timeout = 30

[model]
disable_reasoning = false
```

### Low-Memory Config (Gemini Embeddings)

For servers with limited RAM, use the Gemini embedding provider to eliminate the ~300–500MB fastembed/ONNX overhead:

```toml
[embedding]
provider = "gemini"
api_key = "your_gemini_api_key"
```

## Development

### Build

```bash
cargo build
```

### Run with Debug Logging

```bash
RUST_LOG=debug cargo run
```

### Format & Lint

```bash
cargo fmt
cargo clippy
```

### Using Bacon

Install [bacon](https://dystroy.org/bacon/) for continuous checking:

```bash
cargo install bacon
bacon
```

Keybindings: `c` check, `l` clippy, `t` test, `r` run, `d` doc

## Troubleshooting

### Embedding model download fails (local provider)

The model is downloaded from HuggingFace on first run. Ensure network access to `huggingface.co`. The model is cached in `data/models/` — delete this directory to force re-download.

### Gemini embedding errors

- Verify your API key is valid: `curl "https://generativelanguage.googleapis.com/v1beta/models?key=YOUR_KEY"`
- Check free tier quota at [Google AI Studio](https://aistudio.google.com/)
- If using `[api].key` as fallback, ensure the key has Gemini embedding access

### Switching embedding providers

When switching between `local` (384d) and `gemini` (768d), delete the vector index since dimensions are incompatible:

```bash
rm data/conversations.usearch
# Restart — index rebuilds as new conversations come in
```

### Database errors

If the database becomes corrupted, delete `data/memory.db` and `data/conversations.usearch` to start fresh. All conversation history will be lost, but important facts can be re-added.

### Auto-update not working

**Linux:**

```bash
# Check timer is active
systemctl --user list-timers | grep rustclaw

# Run update manually and check output
systemctl --user start rustclaw-update
journalctl --user -u rustclaw-update --no-pager -n 20
```

**macOS:**

```bash
# Check agent is loaded
launchctl print gui/$(id -u)/com.rustclaw.update

# Run update manually and check output
launchctl kickstart gui/$(id -u)/com.rustclaw.update
tail -20 ~/Library/Logs/rustclaw/update.log
```

**Both platforms:**

```bash
# Ensure GitHub API is reachable
curl -fsSL https://api.github.com/repos/shimaenaga1123/rustclaw/releases/latest | grep tag_name

# Check updater binary exists
ls -la ~/.local/share/rustclaw/rustclaw-update
```

### Discord bot not responding

1. Check bot has "Message Content Intent" enabled
2. Verify bot has proper permissions in the server
3. Check `RUST_LOG=debug` output for errors

### Commands failing

1. Ensure Docker is running: `docker info`
2. Check the Bun image is accessible: `docker run --rm oven/bun:debian bun --version`
3. Review logs for container creation errors

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## License

MIT License - see [LICENSE](LICENSE) for details

## Acknowledgments

- [NanoClaw](https://github.com/gavrielc/nanoclaw) - Original inspiration
- [Rig](https://github.com/0xPlaygrounds/rig) - AI framework
- [serenity](https://github.com/serenity-rs/serenity) - Discord library
- [usearch](https://github.com/unum-cloud/usearch) - Vector search engine
- [SQLite](https://sqlite.org/) via [sqlx](https://github.com/launchbadge/sqlx) - Database
- [fastembed](https://github.com/Anush008/fastembed-rs) - Local embedding inference
- [Typst](https://typst.app/) - Markup-based typesetting
- [cargo-dist](https://github.com/axodotdev/cargo-dist) - Release automation
- [Google Gemini](https://ai.google.dev/) - Embedding API

## Support

- Issues: [GitHub Issues](https://github.com/shimaenaga1123/rustclaw/issues)
