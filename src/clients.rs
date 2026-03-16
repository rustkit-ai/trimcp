use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Config format ─────────────────────────────────────────────────────────────

/// How a client stores its MCP server list.
#[derive(Debug, Clone, Copy)]
pub enum Format {
    /// `{ "mcpServers": { "<name>": { "command", "args", "env" } } }`
    /// Used by: Claude Code, Claude Desktop, Cursor, Windsurf, Continue.dev
    Mcp,

    /// `{ "servers": { "<name>": { "type", "command", "args" } } }`
    /// Used by: VS Code 1.99+
    VsCode,

    /// `{ "context_servers": { "<name>": { "command": { "path", "args" } } } }`
    /// Used by: Zed
    ZedContext,
}

// ── Client definitions ────────────────────────────────────────────────────────

pub struct ClientDef {
    pub name: &'static str,
    pub config_path: PathBuf,
    pub format: Format,
    pub restart_hint: &'static str,
}

/// A parsed server entry from a client config.
#[derive(Debug, Clone)]
pub struct ServerEntry {
    pub name: String,
    #[allow(dead_code)]
    pub command: Option<String>, // None for HTTP servers
    #[allow(dead_code)]
    pub args: Vec<String>,
    #[allow(dead_code)]
    pub env: HashMap<String, String>,
    pub status: ServerStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerStatus {
    Proxied,  // command == "trimcp"
    Direct,   // normal command, active
    Disabled, // disabled: true
    Http,     // serverUrl or url field
}

/// A detected client with its parsed server list.
pub struct DetectedClient {
    pub def: ClientDef,
    pub servers: Vec<ServerEntry>, // empty if file not found or no mcpServers
}

// ── Client registry ───────────────────────────────────────────────────────────

/// Build a VS Code globalStorage path for a given extension and settings file.
fn vscode_ext_path(vscode_base: &Path, extension_id: &str, file: &str) -> PathBuf {
    vscode_base
        .join("User")
        .join("globalStorage")
        .join(extension_id)
        .join("settings")
        .join(file)
}

/// List all client definitions (regardless of whether config exists).
pub fn all_client_defs() -> Vec<ClientDef> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let home = PathBuf::from(&home);

    let mut candidates: Vec<ClientDef> = vec![
        // ── Claude Code ───────────────────────────────────────────────────────
        ClientDef {
            name: "Claude Code",
            config_path: home.join(".claude").join("settings.json"),
            format: Format::Mcp,
            restart_hint: "Restart Claude Code to apply changes.",
        },
        // ── Cursor ────────────────────────────────────────────────────────────
        ClientDef {
            name: "Cursor",
            config_path: home.join(".cursor").join("mcp.json"),
            format: Format::Mcp,
            restart_hint: "Restart Cursor to apply changes.",
        },
        // ── Windsurf ──────────────────────────────────────────────────────────
        ClientDef {
            name: "Windsurf",
            config_path: home
                .join(".codeium")
                .join("windsurf")
                .join("mcp_config.json"),
            format: Format::Mcp,
            restart_hint: "Restart Windsurf to apply changes.",
        },
        // ── Zed ───────────────────────────────────────────────────────────────
        ClientDef {
            name: "Zed",
            config_path: home.join(".config").join("zed").join("settings.json"),
            format: Format::ZedContext,
            restart_hint: "Restart Zed to apply changes.",
        },
        // ── Continue.dev ──────────────────────────────────────────────────────
        ClientDef {
            name: "Continue.dev",
            config_path: home.join(".continue").join("config.json"),
            format: Format::Mcp,
            restart_hint: "Restart VS Code / JetBrains to apply changes.",
        },
    ];

    // ── OS-specific paths ─────────────────────────────────────────────────────

    #[cfg(target_os = "macos")]
    {
        let app_support = home.join("Library").join("Application Support");
        let vscode = app_support.join("Code");
        let vscode_insiders = app_support.join("Code - Insiders");

        candidates.push(ClientDef {
            name: "Claude Desktop",
            config_path: app_support
                .join("Claude")
                .join("claude_desktop_config.json"),
            format: Format::Mcp,
            restart_hint: "Restart Claude Desktop to apply changes.",
        });
        candidates.push(ClientDef {
            name: "VS Code",
            config_path: vscode.join("User").join("mcp.json"),
            format: Format::VsCode,
            restart_hint: "Restart VS Code to apply changes.",
        });
        candidates.push(ClientDef {
            name: "VS Code Insiders",
            config_path: vscode_insiders.join("User").join("mcp.json"),
            format: Format::VsCode,
            restart_hint: "Restart VS Code Insiders to apply changes.",
        });
        candidates.push(ClientDef {
            name: "Cline",
            config_path: vscode_ext_path(
                &vscode,
                "saoudrizwan.claude-dev",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code to apply Cline changes.",
        });
        candidates.push(ClientDef {
            name: "Roo Code",
            config_path: vscode_ext_path(
                &vscode,
                "rooveterinaryinc.roo-cline",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code to apply Roo Code changes.",
        });
        candidates.push(ClientDef {
            name: "Cline (Insiders)",
            config_path: vscode_ext_path(
                &vscode_insiders,
                "saoudrizwan.claude-dev",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code Insiders to apply Cline changes.",
        });
        candidates.push(ClientDef {
            name: "Roo Code (Insiders)",
            config_path: vscode_ext_path(
                &vscode_insiders,
                "rooveterinaryinc.roo-cline",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code Insiders to apply Roo Code changes.",
        });
    }

    #[cfg(target_os = "linux")]
    {
        let config = home.join(".config");
        let vscode = config.join("Code");
        let vscode_insiders = config.join("Code - Insiders");

        candidates.push(ClientDef {
            name: "Claude Desktop",
            config_path: config.join("Claude").join("claude_desktop_config.json"),
            format: Format::Mcp,
            restart_hint: "Restart Claude Desktop to apply changes.",
        });
        candidates.push(ClientDef {
            name: "VS Code",
            config_path: vscode.join("User").join("mcp.json"),
            format: Format::VsCode,
            restart_hint: "Restart VS Code to apply changes.",
        });
        candidates.push(ClientDef {
            name: "VS Code Insiders",
            config_path: vscode_insiders.join("User").join("mcp.json"),
            format: Format::VsCode,
            restart_hint: "Restart VS Code Insiders to apply changes.",
        });
        candidates.push(ClientDef {
            name: "Cline",
            config_path: vscode_ext_path(
                &vscode,
                "saoudrizwan.claude-dev",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code to apply Cline changes.",
        });
        candidates.push(ClientDef {
            name: "Roo Code",
            config_path: vscode_ext_path(
                &vscode,
                "rooveterinaryinc.roo-cline",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code to apply Roo Code changes.",
        });
        candidates.push(ClientDef {
            name: "Cline (Insiders)",
            config_path: vscode_ext_path(
                &vscode_insiders,
                "saoudrizwan.claude-dev",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code Insiders to apply Cline changes.",
        });
        candidates.push(ClientDef {
            name: "Roo Code (Insiders)",
            config_path: vscode_ext_path(
                &vscode_insiders,
                "rooveterinaryinc.roo-cline",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code Insiders to apply Roo Code changes.",
        });
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| home.display().to_string()));
        let vscode = appdata.join("Code");
        let vscode_insiders = appdata.join("Code - Insiders");

        candidates.push(ClientDef {
            name: "Claude Desktop",
            config_path: appdata.join("Claude").join("claude_desktop_config.json"),
            format: Format::Mcp,
            restart_hint: "Restart Claude Desktop to apply changes.",
        });
        candidates.push(ClientDef {
            name: "VS Code",
            config_path: vscode.join("User").join("mcp.json"),
            format: Format::VsCode,
            restart_hint: "Restart VS Code to apply changes.",
        });
        candidates.push(ClientDef {
            name: "VS Code Insiders",
            config_path: vscode_insiders.join("User").join("mcp.json"),
            format: Format::VsCode,
            restart_hint: "Restart VS Code Insiders to apply changes.",
        });
        candidates.push(ClientDef {
            name: "Cline",
            config_path: vscode_ext_path(
                &vscode,
                "saoudrizwan.claude-dev",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code to apply Cline changes.",
        });
        candidates.push(ClientDef {
            name: "Roo Code",
            config_path: vscode_ext_path(
                &vscode,
                "rooveterinaryinc.roo-cline",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code to apply Roo Code changes.",
        });
        candidates.push(ClientDef {
            name: "Cline (Insiders)",
            config_path: vscode_ext_path(
                &vscode_insiders,
                "saoudrizwan.claude-dev",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code Insiders to apply Cline changes.",
        });
        candidates.push(ClientDef {
            name: "Roo Code (Insiders)",
            config_path: vscode_ext_path(
                &vscode_insiders,
                "rooveterinaryinc.roo-cline",
                "cline_mcp_settings.json",
            ),
            format: Format::Mcp,
            restart_hint: "Restart VS Code Insiders to apply Roo Code changes.",
        });
    }

    candidates
}

// ── Server parsing ────────────────────────────────────────────────────────────

fn parse_server_status(entry: &Value, command: Option<&str>) -> ServerStatus {
    // HTTP-based
    if entry.get("serverUrl").is_some() || entry.get("url").is_some() {
        return ServerStatus::Http;
    }
    // Disabled
    if entry
        .get("disabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return ServerStatus::Disabled;
    }
    // Proxied
    if command == Some("trimcp") {
        return ServerStatus::Proxied;
    }
    ServerStatus::Direct
}

fn parse_env(entry: &Value) -> HashMap<String, String> {
    entry
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_args(entry: &Value) -> Vec<String> {
    entry
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_mcp_servers(json: &Value) -> Vec<ServerEntry> {
    let Some(servers) = json.get("mcpServers").and_then(|v| v.as_object()) else {
        return vec![];
    };

    servers
        .iter()
        .map(|(name, entry)| {
            let command = entry.get("command").and_then(|v| v.as_str());
            let status = parse_server_status(entry, command);
            ServerEntry {
                name: name.clone(),
                command: command.map(str::to_string),
                args: parse_args(entry),
                env: parse_env(entry),
                status,
            }
        })
        .collect()
}

fn parse_vscode_servers(json: &Value) -> Vec<ServerEntry> {
    let Some(servers) = json.get("servers").and_then(|v| v.as_object()) else {
        return vec![];
    };

    servers
        .iter()
        .filter_map(|(name, entry)| {
            // Skip SSE servers
            if entry.get("type").and_then(|v| v.as_str()) == Some("sse") {
                return None;
            }
            let command = entry.get("command").and_then(|v| v.as_str());
            let status = parse_server_status(entry, command);
            Some(ServerEntry {
                name: name.clone(),
                command: command.map(str::to_string),
                args: parse_args(entry),
                env: parse_env(entry),
                status,
            })
        })
        .collect()
}

fn parse_zed_servers(json: &Value) -> Vec<ServerEntry> {
    let Some(servers) = json.get("context_servers").and_then(|v| v.as_object()) else {
        return vec![];
    };

    servers
        .iter()
        .map(|(name, entry)| {
            let cmd_obj = entry.get("command");
            let command = cmd_obj.and_then(|c| c.get("path")).and_then(|v| v.as_str());
            let args: Vec<String> = cmd_obj
                .and_then(|c| c.get("args"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let status = parse_server_status(entry, command);
            ServerEntry {
                name: name.clone(),
                command: command.map(str::to_string),
                args,
                env: parse_env(entry),
                status,
            }
        })
        .collect()
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Detect all known MCP clients and parse their server lists.
pub fn detect_all() -> Vec<DetectedClient> {
    all_client_defs()
        .into_iter()
        .map(|def| {
            let servers = if def.config_path.exists() {
                parse_client_config(&def.config_path, def.format)
            } else {
                vec![]
            };
            DetectedClient { def, servers }
        })
        .collect()
}

fn parse_client_config(path: &Path, format: Format) -> Vec<ServerEntry> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return vec![];
    };
    if raw.trim().is_empty() {
        return vec![];
    }
    let Ok(json) = serde_json::from_str::<Value>(&raw) else {
        return vec![];
    };
    match format {
        Format::Mcp => parse_mcp_servers(&json),
        Format::VsCode => parse_vscode_servers(&json),
        Format::ZedContext => parse_zed_servers(&json),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_mcp_servers_proxied() {
        let json = json!({
            "mcpServers": {
                "my-server": { "command": "trimcp", "args": ["proxy", "my-server"] }
            }
        });
        let entries = parse_mcp_servers(&json);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, ServerStatus::Proxied);
    }

    #[test]
    fn test_parse_mcp_servers_direct() {
        let json = json!({
            "mcpServers": {
                "my-server": { "command": "npx", "args": ["-y", "some-server"] }
            }
        });
        let entries = parse_mcp_servers(&json);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, ServerStatus::Direct);
    }

    #[test]
    fn test_parse_mcp_servers_disabled() {
        let json = json!({
            "mcpServers": {
                "my-server": { "command": "npx", "disabled": true }
            }
        });
        let entries = parse_mcp_servers(&json);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, ServerStatus::Disabled);
    }

    #[test]
    fn test_parse_mcp_servers_http() {
        let json = json!({
            "mcpServers": {
                "remote": { "serverUrl": "https://example.com/mcp" }
            }
        });
        let entries = parse_mcp_servers(&json);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, ServerStatus::Http);
    }

    #[test]
    fn test_parse_vscode_servers_skips_sse() {
        let json = json!({
            "servers": {
                "sse-server": { "type": "sse", "url": "https://example.com" },
                "stdio-server": { "type": "stdio", "command": "npx" }
            }
        });
        let entries = parse_vscode_servers(&json);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "stdio-server");
    }

    #[test]
    fn test_parse_zed_servers() {
        let json = json!({
            "context_servers": {
                "my-zed-server": {
                    "command": { "path": "npx", "args": ["-y", "server"] }
                }
            }
        });
        let entries = parse_zed_servers(&json);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, Some("npx".to_string()));
        assert_eq!(entries[0].args, vec!["-y", "server"]);
        assert_eq!(entries[0].status, ServerStatus::Direct);
    }

    #[test]
    fn test_parse_zed_servers_proxied() {
        let json = json!({
            "context_servers": {
                "my-server": {
                    "command": { "path": "trimcp", "args": ["proxy", "my-server"] }
                }
            }
        });
        let entries = parse_zed_servers(&json);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, ServerStatus::Proxied);
    }

    #[test]
    fn test_parse_mcp_servers_empty() {
        let json = json!({});
        let entries = parse_mcp_servers(&json);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_all_client_defs_returns_candidates() {
        let defs = all_client_defs();
        assert!(!defs.is_empty());
        // Claude Code and Cursor should always be present
        assert!(defs.iter().any(|d| d.name == "Claude Code"));
        assert!(defs.iter().any(|d| d.name == "Cursor"));
    }
}
