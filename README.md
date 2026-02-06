# RustClaw

A lightweight, memory-aware Discord AI assistant powered by Anthropic-compatible APIs, implemented in Rust. Inspired by [nanoclaw](https://github.com/gavrielc/nanoclaw) and [OpenClaw](https://github.com/openclaw/openclaw)

## Features

- **Discord Integration**: Mention-based interaction
- **Anthropic-compatible API**: Works with Claude, Minimax, and other Anthropic-compatible endpoints via [Rig](https://github.com/0xPlaygrounds/rig)
- **Dual Memory System**: Automatic short-term/long-term memory management
- **Tool Calling**: Shell command execution, web search, and memory operations
- **Sandboxed Execution**: Non-owner commands run in Docker containers with mise
- **Brave Search**: Optional web search integration
- **Task Scheduler**: Cron-based task scheduling with persistence

## Prerequisites

- Rust 1.70+
- Discord Bot Token
- Anthropic-compatible API Key (Claude, Minimax, etc.)
- Docker (for sandboxed command execution)
- Brave Search API Key (optional)

## Quick Start

### 1. Clone & Setup

```bash
git clone https://github.com/yourusername/rustclaw
cd rustclaw
cp .env.example .env
```

### 2. Configure Environment

Edit `.env` with your credentials:

```env
DISCORD_TOKEN=your_token
OWNER_ID=your_discord_user_id  # Right-click profile -> Copy User ID
API_KEY=your_api_key
API_URL=https://api.anthropic.com/v1  # or any Anthropic-compatible endpoint
MODEL=claude-3-5-sonnet-20241022  # or your preferred model
BRAVE_API_KEY=your_key  # optional
SANDBOX_IMAGE=jdxcode/mise:latest  # Docker image for sandboxed execution
```

### 3. Create Discord Bot

1. Go to [Discord Developer Portal](https://discord.com/developers/applications)
2. Create New Application → Bot
3. Enable "Message Content Intent"
4. Copy token to `.env`
5. Invite bot with permissions: `View Channels`, `Send Messages`, `Read Message History`

### 4. Run

```bash
cargo run --release
```

## Usage

Mention the bot in Discord to interact:

```
@YourBot What's the weather like today?

@YourBot search for rust async programming

@YourBot run ls -la

@YourBot remember that I prefer dark mode
```

## Memory System

### Short-Term Memory
- Stores recent conversation history
- Auto-compresses at 80% of context limit
- Saved to `data/recent.md`

### Long-Term Memory
- Extracted via AI summarization
- Manually added via `remember` tool
- Saved to `data/memory.md`

### Archives
- Compressed conversations stored in `data/conversations/`
- Named by timestamp: `20260206-143022.md`

## Tools

The bot has access to these tools:

### `run_command`
Execute shell commands:
```
@YourBot run python --version
```

- **Owner**: Commands run directly on the host system
- **Others**: Commands run in a sandboxed Docker container with [mise](https://mise.jdx.dev/) for language runtime management

### `web_search`
Search the web using Brave API:
```
@YourBot search for latest rust news
```

### `remember`
Save important information:
```
@YourBot remember my birthday is January 1st
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
Remove a scheduled task:
```
@YourBot remove schedule abc123
```

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
│   ├── main.rs           # Entry point
│   ├── config.rs         # Configuration
│   ├── agent/
│   │   ├── mod.rs
│   │   ├── rig_agent.rs  # AI agent + Rig integration
│   │   └── tools/        # Tool implementations
│   │       ├── mod.rs
│   │       ├── error.rs
│   │       ├── run_command.rs
│   │       ├── remember.rs
│   │       ├── web_search.rs
│   │       ├── schedule.rs
│   │       ├── unschedule.rs
│   │       └── list_schedules.rs
│   ├── discord/
│   │   ├── mod.rs
│   │   └── bot.rs        # Discord event handler
│   ├── memory/
│   │   ├── mod.rs
│   │   └── manager.rs    # Memory management
│   └── scheduler/
│       ├── mod.rs
│       └── cron.rs       # Task scheduler
└── data/
    ├── recent.md         # Short-term memory
    ├── memory.md         # Long-term memory
    ├── schedules.json    # Scheduled tasks
    └── conversations/    # Archives
```

## Configuration

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DISCORD_TOKEN` | ✅ | - | Discord bot token |
| `OWNER_ID` | ✅ | - | Discord user ID of the bot owner |
| `API_KEY` | ✅ | - | Anthropic-compatible API key |
| `API_URL` | ❌ | `https://api.anthropic.com/v1` | API endpoint (Claude, Minimax, etc.) |
| `MODEL` | ❌ | `claude-3-5-sonnet-20241022` | Model name |
| `BRAVE_API_KEY` | ❌ | - | Brave Search API key |
| `DATA_DIR` | ❌ | `data` | Data directory path |
| `CONTEXT_LIMIT` | ❌ | `128000` | Token limit |
| `COMMAND_TIMEOUT` | ❌ | `30` | Command timeout (seconds) |
| `SANDBOX_IMAGE` | ❌ | `jdxcode/mise:latest` | Docker image for sandboxed execution |
| `RUST_LOG` | ❌ | `info` | Log level |

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

## Troubleshooting

### Memory not persisting

Check that `DATA_DIR` is writable:
```bash
ls -la data/
```

### Discord bot not responding

1. Check bot has "Message Content Intent" enabled
2. Verify bot has proper permissions in the server
3. Check `RUST_LOG=debug` output for errors

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

## Support

- Issues: [GitHub Issues](https://github.com/shimaenaga1123/rustclaw/issues)
