<h1 align="center">trimcp</h1>

<p align="center">
  MCP proxy that cuts LLM token costs by 60–90% through lossless compression and caching.
</p>

<p align="center">
  <a href="https://github.com/rustkit-ai/trimcp/actions/workflows/ci.yml"><img src="https://github.com/rustkit-ai/trimcp/actions/workflows/ci.yml/badge.svg" alt="CI"/></a>
  <a href="https://crates.io/crates/trimcp"><img src="https://img.shields.io/crates/v/trimcp.svg" alt="crates.io"/></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"/></a>
</p>

---

**MCP tool outputs are full of noise.** Pretty-printed JSON, ANSI color codes, repeated log lines, inline comments — none of it adds information, all of it costs tokens. On a busy session with filesystem and git tools, this quietly burns 30–50% of your context budget before the model even starts thinking.

`trimcp` sits transparently between your LLM client and every MCP server. It strips the noise, caches identical calls, and reports exactly what it saved.

```
LLM client (Claude Code, Cursor, claude_desktop…)
        ↓
    trimcp          ← strips ANSI, minifies JSON, deduplicates lines, caches
     ↙    ↓    ↘
  MCP1  MCP2  MCP3
```

```
[trimcp] session summary
  Requests    : 47
  Tokens in   : 84,312
  Tokens out  : 31,847  (62% smaller)
  Cache hits  : 12 / 47
```

Zero configuration required — it works out of the box.

---

## Install

```bash
cargo install trimcp
```

---

## Quick start

```bash
# Register your MCP servers once
trimcp add filesystem -- npx -y @modelcontextprotocol/server-filesystem /tmp
trimcp add github    -- npx -y @modelcontextprotocol/server-github

# Point your LLM client at trimcp instead of the server directly
trimcp proxy filesystem
```

---

## LLM client configuration

Replace your server commands with `trimcp proxy <name>`. Nothing else changes.

**Claude Code** (`~/.claude/settings.json`):
```json
{
  "mcpServers": {
    "filesystem": { "command": "trimcp", "args": ["proxy", "filesystem"] },
    "github":     { "command": "trimcp", "args": ["proxy", "github"] }
  }
}
```

**Cursor / claude_desktop** (`claude_desktop_config.json`):
```json
{
  "mcpServers": {
    "filesystem": { "command": "trimcp", "args": ["proxy", "filesystem"] }
  }
}
```

---

## What gets compressed

| Strategy | What it removes | Typical saving |
|---|---|---|
| `strip_ansi` | Terminal color/cursor codes (`\x1b[…`) | ~5% |
| `compact_json` | Whitespace in pretty-printed JSON | ~30% |
| `dedup` | Consecutive identical lines → `line (xN)` | ~40% on logs |
| `minify` | Trailing whitespace, extra blank lines | ~5% |
| `strip_comments` | `//` full-line comments in code blocks | ~10% |

All strategies are lossless — no information is truncated or dropped.

---

## Configuration

`~/.config/trimcp/config.toml` — created automatically on first `add`.

```toml
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[compression]
enabled       = true   # master switch
strip_ansi    = true
compact_json  = true
dedup         = true
minify        = true
strip_comments = false

[cache]
enabled  = true
ttl_secs = 300    # cache lifetime in seconds

[metrics]
enabled  = true   # print summary to stderr at session end
realtime = false  # print running total after each tool call
```

---

## Optional: semantic code context (`semtree` feature)

Enable the `semtree` feature to inject relevant code snippets into every `tools/call` response. The model sees the context it needs without you having to ask for it.

```toml
# Cargo.toml
trimcp = { version = "0.1", features = ["semtree"] }
```

```toml
# ~/.config/trimcp/config.toml
[servers.my-server]
semtree_codebase = "/path/to/your/project"
semtree_top_k    = 3

# Index the codebase once
# trimcp semtree-index my-server
```

---

## CLI

```
trimcp add <name> -- <command> [args…]   Register a server
trimcp remove <name>                      Remove a server
trimcp list                               List configured servers
trimcp proxy <name>                       Run as proxy (used by LLM clients)
trimcp stats                              Show compression statistics
trimcp knowledge                          Inspect the semantic knowledge store
trimcp semtree-index <name>               Index a codebase (--features semtree)
```

---

## License

MIT — see [LICENSE](LICENSE)
