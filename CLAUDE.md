# rustkit-mcp — Claude Guidelines

## Project Overview

`rustkit-mcp` is an MCP (Model Context Protocol) proxy written in Rust.
It sits between an LLM client and upstream MCP servers to reduce token costs
by compressing, filtering, and caching tool outputs.

```
LLM Client (Claude, Cursor...)
        ↓
  [rustkit-mcp]   ← this project
   ↙    ↓    ↘
MCP1  MCP2  MCP3
```

## Architecture

- **Transport**: stdio (v0.1), HTTP/SSE (later)
- **Protocol**: JSON-RPC 2.0 (MCP spec)
- **Core modules**:
  - `proxy` — request forwarding to upstream MCP servers
  - `compress` — output compression strategies (truncate, filter, summarize)
  - `cache` — TTL-based result caching
  - `metrics` — token savings reporting

## Conventions

### Commits
Follow [Conventional Commits](https://www.conventionalcommits.org/):
```
feat: add stdio transport layer
fix: handle empty tool output gracefully
refactor: extract compression strategy trait
chore: update dependencies
docs: add proxy architecture diagram
test: add compression unit tests
```

### Code Style
- `cargo fmt` before every commit
- `cargo clippy -- -D warnings` must pass
- No `unwrap()` in production code — use `?` or explicit error handling
- Prefer `thiserror` for error types, `anyhow` for application-level errors
- Document all public APIs with `///` doc comments

### Error Handling
- Define domain errors in `src/error.rs` with `thiserror`
- Propagate errors with `?`, never `.unwrap()` or `.expect()` in non-test code

### Testing
- Unit tests in the same file as the code (`#[cfg(test)]`)
- Integration tests in `tests/`
- Test names: `test_<what>_<when>_<expected>`

## Backlog

Tickets are in `/backlog` (local only, not committed). See `backlog/README.md`.

## Development Setup

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```
