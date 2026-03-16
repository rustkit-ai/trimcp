# trimcp

[![CI](https://github.com/rustkit-ai/trimcp/actions/workflows/ci.yml/badge.svg)](https://github.com/rustkit-ai/trimcp/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/trimcp.svg)](https://crates.io/crates/trimcp)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

MCP proxy that reduces LLM token costs by **60–90%** through output compression and TTL caching.

```
LLM Client (Claude Code, Cursor, claude_desktop…)
        ↓
  trimcp          ← strips ANSI, minifies JSON, deduplicates lines
   ↙    ↓    ↘
MCP1  MCP2  MCP3
```

## Why

MCP tool outputs are often verbose: pretty-printed JSON, ANSI color codes, repeated lines, inline comments. All of that costs tokens without adding information. `trimcp` sits between your LLM client and the upstream servers and applies lossless compression before the output reaches the model.

## Install

```bash
cargo install trimcp
```

## Quick start

```bash
# Register your MCP servers
trimcp add filesystem -- npx -y @modelcontextprotocol/server-filesystem /tmp
trimcp add github    -- npx -y @modelcontextprotocol/server-github

# Check what's configured
trimcp list

# Use in proxy mode (called by your LLM client)
trimcp proxy filesystem
```

## LLM client configuration

### Claude Code (`~/.claude/settings.json`)

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "trimcp",
      "args": ["proxy", "filesystem"]
    },
    "github": {
      "command": "trimcp",
      "args": ["proxy", "github"]
    }
  }
}
```

### Cursor / claude_desktop_config.json

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "trimcp",
      "args": ["proxy", "filesystem"]
    }
  }
}
```

## Configuration

Config is stored at `~/.config/trimcp/config.toml` (created automatically on first `add`).

```toml
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[compression]
enabled = true        # master switch
strip_ansi = true     # remove terminal color codes
compact_json = true   # minify pretty-printed JSON
strip_comments = false # remove // comments inside code blocks
dedup = true          # collapse repeated lines into "line (xN)"
minify = true         # trim trailing whitespace, collapse blank lines

[metrics]
enabled = true        # print summary to stderr at session end
realtime = false      # print running total after each tool call

[cache]
enabled = true        # TTL cache for identical tool call results
ttl_secs = 300        # cache lifetime in seconds
```

## Compression strategies

| Strategy | What it removes | Example savings |
|---|---|---|
| `StripAnsi` | Terminal color/cursor codes (`\x1b[…`) | ~5% on colored output |
| `CompactJson` | Whitespace in pretty-printed JSON | ~30% on JSON responses |
| `Dedup` | Consecutive identical lines → `line (xN)` | ~40% on log output |
| `Minify` | Trailing whitespace, extra blank lines | ~5% everywhere |
| `StripComments` | `//` full-line comments in code blocks | ~10% on documented code |

Strategies run in pipeline order. None of them truncate or lose information.

## CLI reference

```
trimcp add <name> -- <command> [args...]   Add a server to the config
trimcp remove <name>                        Remove a server
trimcp list                                 List configured servers
trimcp proxy <name> [--metrics]             Run as proxy (used by LLM clients)
```

## Development

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

## License

MIT
