# RustClaw

A lightweight, memory-aware Discord AI assistant powered by Anthropic-compatible APIs, implemented in Rust. Inspired by [nanoclaw](https://github.com/gavrielc/nanoclaw) and [OpenClaw](https://github.com/openclaw/openclaw)

## Features

- **Discord Integration**: Mention-based interaction
- **Anthropic-compatible API**: Works with Claude, Minimax, and other Anthropic-compatible endpoints via [Rig](https://github.com/0xPlaygrounds/rig)
- **Vector Memory System**: LanceDB-powered semantic memory with local embeddings (multilingual-e5-small)
    - **Important Facts**: Owner-managed persistent facts, always included in context
    - **Conversation History**: Every turn stored with embeddings for semantic retrieval
    - **Hybrid Context**: Recent 20 turns + top 10 semantically similar past conversations
- **Tool Calling**: Shell command execution, web search, and memory operations
- **Sandboxed Execution**: All commands run in isolated Debian Docker containers with Bun runtime
- **Owner Permission System**: Owner/non-owner distinction with AI-level awareness for safe multi-user operation
- **Brave Search**: Optional web search integration
- **Task Scheduler**: Cron-based task scheduling with persistence

## Prerequisites

- Rust 1.70+
- Discord Bot Token
- Anthropic-compatible API Key (Claude, Minimax, etc.)
- Docker (required for all command execution)
- Brave Search API Key (optional)

## Quick Start

### 1. Clone & Setup

```bash
git clone https://github.com/yourusername/rustclaw
cd rustclaw
cp config.example.toml config.toml
```

### 2. Configure

Edit `config.toml` with your credentials. See `config.example.toml` for all options.

### 3. Create Discord Bot

1. Go to [Discord Developer Portal](https://discord.com/developers/applications)
2. Create New Application → Bot
3. Enable "Message Content Intent"
4. Copy token to `config.toml`
5. Invite bot with permissions: `View Channels`, `Send Messages`, `Read Message History`

### 4. Prepare Docker

Ensure Docker is running. The bot will automatically pull the `oven/bun:debian` image on first command execution.

```bash
docker pull oven/bun:debian  # optional, speeds up first run
```

### 5. Run

```bash
cargo run --release
```

On first run, the embedding model (~130MB) will be downloaded from HuggingFace automatically.

## Installation (Linux Service)

```bash
./install.sh
```

Update (stops service, rebuilds, restarts):

```bash
./install.sh
```

Manage service:

```bash
systemctl --user start rustclaw
systemctl --user stop rustclaw
systemctl --user status rustclaw
journalctl --user -u rustclaw -f
```

Enable auto-start on boot:

```bash
sudo loginctl enable-linger $USER
```

## Usage

Mention the bot in Discord to interact:

```
@YourBot What's the weather like today?

@YourBot search for rust async programming

@YourBot run ls -la

@YourBot remember that I prefer dark mode
```

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

## Memory System

RustClaw uses a vector-based memory system powered by **LanceDB** and **fastembed** (multilingual-e5-small) for local, GPU-free semantic search.

### Important Facts (`important` table)
- Owner-managed key facts (preferences, dates, decisions)
- Always loaded in full into every conversation context
- CRUD via `important_add`, `important_list`, `important_delete` tools (owner only)
- Stored with vector embeddings for future extensibility

### Conversation History (`long_term_memory` table)
- Every conversation turn (user input + assistant response) stored immediately
- Each turn is embedded for semantic retrieval
- No compression or summarization — original text preserved
- Context includes:
    - **Recent 20 turns**: maintains conversation flow and continuity
    - **Semantic top 10**: past turns most relevant to the current input (deduplicated against recent)

### How Context is Built

```
┌─────────────────────────────────┐
│  # Important Facts              │  ← All entries, always
│  - User prefers dark mode       │
│  - Birthday is January 1st      │
├─────────────────────────────────┤
│  # Recent Conversations         │  ← Last 20 turns (chronological)
│  User: ...                      │
│  Assistant: ...                 │
├─────────────────────────────────┤
│  # Related Past Conversations   │  ← Top 10 semantic matches
│  User: ...                      │     (excluding recent turns)
│  Assistant: ...                 │
└─────────────────────────────────┘
```

### Data Storage

All data is stored in `data/lancedb/` using the Lance columnar format. No `.md` files, no plain-text archives.

The embedding model is cached in `data/models/` after the initial download.

## Tools

The bot has access to these tools:

### `run_command`
Execute shell commands in an isolated Debian Docker container with Bun runtime:
```
@YourBot run bun --version
```

- All users' commands run inside `oven/bun:debian` containers
- Bun and Node.js are pre-installed
- Use `apt-get install` to install additional packages within the container

### `reset_container`
Reset the Docker sandbox container (owner only):
```
@YourBot reset the container
```

- Stops and removes the current container
- Clears the workspace directory
- A fresh container is created on the next command

### `web_search`
Search the web using Brave API:
```
@YourBot search for latest rust news
```

### `important_add`
Save important information to persistent memory (owner only):
```
@YourBot remember my birthday is January 1st
```

### `important_list`
List all stored important facts:
```
@YourBot show all important facts
```

### `important_delete`
Remove an important entry by ID (owner only):
```
@YourBot delete important entry abc123
```

### `weather`
Get current weather and forecast for any location:
```
@YourBot what's the weather in Seoul?
```

### `send_file`
Send files from the Docker workspace as Discord attachments:
```
@YourBot create a script and send it to me
```

### `schedule`
Schedule recurring tasks with cron expressions:
```
@YourBot schedule a daily weather check at 9am
```

### `list_schedules`
List all scheduled tasks:
```
@YourBot show my scheduled tasks
```

### `unschedule`
Remove a scheduled task (owner only):
```
@YourBot remove schedule abc123
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
│   ├── main.rs           # Entry point with graceful shutdown
│   ├── config.rs         # Configuration
│   ├── utils.rs          # Shared utilities (split_message, etc.)
│   ├── embeddings.rs     # Local embedding service (fastembed, multilingual-e5-small)
│   ├── vectordb.rs       # LanceDB wrapper (long_term_memory + important tables)
│   ├── agent.rs          # AI agent + Rig integration + cached API client
│   ├── tools/            # Tool implementations
│   ├── discord.rs        # Discord event handler
│   ├── memory.rs         # Memory manager (delegates to VectorDb)
│   └── scheduler.rs      # Task scheduler
└── data/
    ├── lancedb/          # Vector database (long_term_memory + important)
    ├── models/           # Cached embedding model (~130MB)
    ├── workspace/        # Docker sandbox mount
    └── schedules.json    # Scheduled tasks
```

## Configuration

See `config.example.toml` for all available options.

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

### Embedding model download fails

The model is downloaded from HuggingFace on first run. Ensure network access to `huggingface.co`. The model is cached in `data/models/` — delete this directory to force re-download.

### LanceDB errors

If the database becomes corrupted, delete `data/lancedb/` to start fresh. All conversation history will be lost, but important facts can be re-added.

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
- [LanceDB](https://lancedb.com/) - Vector database
- [fastembed](https://github.com/Anush008/fastembed-rs) - Local embedding inference

## Support

- Issues: [GitHub Issues](https://github.com/shimaenaga1123/rustclaw/issues)
