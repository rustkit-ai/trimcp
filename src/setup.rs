use crate::config::{Config, ServerConfig};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Config format ─────────────────────────────────────────────────────────────

/// How a client stores its MCP server list.
#[derive(Debug, Clone, Copy)]
enum Format {
    /// `{ "mcpServers": { "<name>": { "command", "args", "env" } } }`
    /// Used by: Claude Code, Claude Desktop, Cursor, Windsurf, Continue.dev
    McpServers,

    /// `{ "servers": { "<name>": { "type", "command", "args" } } }`
    /// Used by: VS Code 1.99+
    VsCodeServers,

    /// `{ "context_servers": { "<name>": { "command": { "path", "args" } } } }`
    /// Used by: Zed
    ZedContextServers,
}

// ── Client registry ───────────────────────────────────────────────────────────

struct Client {
    name: &'static str,
    config_path: PathBuf,
    format: Format,
    restart_hint: &'static str,
}

fn detect_clients() -> Vec<Client> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let home = PathBuf::from(&home);

    let mut candidates: Vec<Client> = vec![
        // ── Claude Code ───────────────────────────────────────────────────────
        Client {
            name: "Claude Code",
            config_path: home.join(".claude").join("settings.json"),
            format: Format::McpServers,
            restart_hint: "Restart Claude Code to apply changes.",
        },
        // ── Cursor ────────────────────────────────────────────────────────────
        Client {
            name: "Cursor",
            config_path: home.join(".cursor").join("mcp.json"),
            format: Format::McpServers,
            restart_hint: "Restart Cursor to apply changes.",
        },
        // ── Windsurf ──────────────────────────────────────────────────────────
        Client {
            name: "Windsurf",
            config_path: home
                .join(".codeium")
                .join("windsurf")
                .join("mcp_config.json"),
            format: Format::McpServers,
            restart_hint: "Restart Windsurf to apply changes.",
        },
        // ── Zed ───────────────────────────────────────────────────────────────
        Client {
            name: "Zed",
            config_path: home.join(".config").join("zed").join("settings.json"),
            format: Format::ZedContextServers,
            restart_hint: "Restart Zed to apply changes.",
        },
        // ── Continue.dev ──────────────────────────────────────────────────────
        Client {
            name: "Continue.dev",
            config_path: home.join(".continue").join("config.json"),
            format: Format::McpServers,
            restart_hint: "Restart VS Code / JetBrains to apply changes.",
        },
    ];

    // ── OS-specific paths ─────────────────────────────────────────────────────

    #[cfg(target_os = "macos")]
    {
        let app_support = home.join("Library").join("Application Support");

        candidates.push(Client {
            name: "Claude Desktop",
            config_path: app_support.join("Claude").join("claude_desktop_config.json"),
            format: Format::McpServers,
            restart_hint: "Restart Claude Desktop to apply changes.",
        });
        candidates.push(Client {
            name: "VS Code",
            config_path: app_support.join("Code").join("User").join("mcp.json"),
            format: Format::VsCodeServers,
            restart_hint: "Restart VS Code to apply changes.",
        });
        candidates.push(Client {
            name: "VS Code Insiders",
            config_path: app_support
                .join("Code - Insiders")
                .join("User")
                .join("mcp.json"),
            format: Format::VsCodeServers,
            restart_hint: "Restart VS Code Insiders to apply changes.",
        });
    }

    #[cfg(target_os = "linux")]
    {
        let config = home.join(".config");

        candidates.push(Client {
            name: "Claude Desktop",
            config_path: config.join("Claude").join("claude_desktop_config.json"),
            format: Format::McpServers,
            restart_hint: "Restart Claude Desktop to apply changes.",
        });
        candidates.push(Client {
            name: "VS Code",
            config_path: config.join("Code").join("User").join("mcp.json"),
            format: Format::VsCodeServers,
            restart_hint: "Restart VS Code to apply changes.",
        });
        candidates.push(Client {
            name: "VS Code Insiders",
            config_path: config
                .join("Code - Insiders")
                .join("User")
                .join("mcp.json"),
            format: Format::VsCodeServers,
            restart_hint: "Restart VS Code Insiders to apply changes.",
        });
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| home.display().to_string()),
        );
        candidates.push(Client {
            name: "Claude Desktop",
            config_path: appdata.join("Claude").join("claude_desktop_config.json"),
            format: Format::McpServers,
            restart_hint: "Restart Claude Desktop to apply changes.",
        });
        candidates.push(Client {
            name: "VS Code",
            config_path: appdata.join("Code").join("User").join("mcp.json"),
            format: Format::VsCodeServers,
            restart_hint: "Restart VS Code to apply changes.",
        });
        candidates.push(Client {
            name: "VS Code Insiders",
            config_path: appdata
                .join("Code - Insiders")
                .join("User")
                .join("mcp.json"),
            format: Format::VsCodeServers,
            restart_hint: "Restart VS Code Insiders to apply changes.",
        });
    }

    candidates
        .into_iter()
        .filter(|c| c.config_path.exists())
        .collect()
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(config_path: &Path) -> Result<()> {
    let clients = detect_clients();

    if clients.is_empty() {
        println!("No MCP clients found.");
        println!("Supported: Claude Code, Claude Desktop, Cursor, Windsurf, VS Code, Zed, Continue.dev");
        println!();
        println!("Add servers manually with: trimcp add <name> -- <command> [args...]");
        return Ok(());
    }

    let mut trimcp_config = Config::load(config_path)?;
    let mut total_imported = 0usize;
    let mut restarted: Vec<&str> = Vec::new();

    for client in &clients {
        println!("{} ({})", client.name, client.config_path.display());

        let raw = std::fs::read_to_string(&client.config_path)?;
        let mut json: Value = serde_json::from_str(&raw)?;

        let imported = match client.format {
            Format::McpServers => {
                process_mcp_servers(&mut json, &mut trimcp_config, client.name)?
            }
            Format::VsCodeServers => {
                process_vscode_servers(&mut json, &mut trimcp_config, client.name)?
            }
            Format::ZedContextServers => {
                process_zed_servers(&mut json, &mut trimcp_config, client.name)?
            }
        };

        if imported > 0 {
            let updated = serde_json::to_string_pretty(&json)?;
            std::fs::write(&client.config_path, updated)?;
            println!("  → {} updated", client.config_path.display());
            total_imported += imported;
            restarted.push(client.restart_hint);
        }

        println!();
    }

    if total_imported > 0 {
        trimcp_config.save(config_path)?;
        println!(
            "{} server{} imported → {}",
            total_imported,
            if total_imported == 1 { "" } else { "s" },
            config_path.display()
        );
        println!();
        for hint in restarted {
            println!("→ {hint}");
        }
    } else {
        println!("Nothing to import — all servers already proxied or skipped.");
    }

    Ok(())
}

// ── Format handlers ───────────────────────────────────────────────────────────

/// Handle `{ "mcpServers": { "<name>": { "command", "args", "env" } } }`
fn process_mcp_servers(
    json: &mut Value,
    trimcp_config: &mut Config,
    client_name: &str,
) -> Result<usize> {
    let Some(servers) = json
        .get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
    else {
        println!("  (no mcpServers found)");
        return Ok(0);
    };

    let mut imported = 0;

    for (name, entry) in servers.iter_mut() {
        if let Some(n) = import_stdio_entry(name, entry, trimcp_config, client_name) {
            *entry = serde_json::json!({ "command": "trimcp", "args": ["proxy", n] });
            imported += 1;
        }
    }

    Ok(imported)
}

/// Handle `{ "servers": { "<name>": { "type", "command", "args" } } }` (VS Code 1.99+)
fn process_vscode_servers(
    json: &mut Value,
    trimcp_config: &mut Config,
    client_name: &str,
) -> Result<usize> {
    let Some(servers) = json
        .get_mut("servers")
        .and_then(|v| v.as_object_mut())
    else {
        println!("  (no servers found)");
        return Ok(0);
    };

    let mut imported = 0;

    for (name, entry) in servers.iter_mut() {
        // VS Code only supports stdio type for local servers
        if entry.get("type").and_then(|v| v.as_str()) == Some("sse") {
            println!("  - {name:<20} skipped (SSE server, not supported)");
            continue;
        }

        if let Some(n) = import_stdio_entry(name, entry, trimcp_config, client_name) {
            *entry = serde_json::json!({
                "type": "stdio",
                "command": "trimcp",
                "args": ["proxy", n]
            });
            imported += 1;
        }
    }

    Ok(imported)
}

/// Handle `{ "context_servers": { "<name>": { "command": { "path", "args" } } } }` (Zed)
fn process_zed_servers(
    json: &mut Value,
    trimcp_config: &mut Config,
    _client_name: &str,
) -> Result<usize> {
    let Some(servers) = json
        .get_mut("context_servers")
        .and_then(|v| v.as_object_mut())
    else {
        println!("  (no context_servers found)");
        return Ok(0);
    };

    let mut imported = 0;

    for (name, entry) in servers.iter_mut() {
        // Already proxied
        if entry
            .get("command")
            .and_then(|c| c.get("path"))
            .and_then(|v| v.as_str())
            == Some("trimcp")
        {
            println!("  ~ {name:<20} already proxied");
            continue;
        }

        let Some(cmd_obj) = entry.get("command") else {
            println!("  ? {name:<20} skipped (no command field)");
            continue;
        };

        let Some(command) = cmd_obj.get("path").and_then(|v| v.as_str()) else {
            println!("  ? {name:<20} skipped (no command.path field)");
            continue;
        };

        let args: Vec<String> = cmd_obj
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let env: HashMap<String, String> = entry
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let display_cmd = if args.is_empty() {
            command.to_string()
        } else {
            format!("{} {}", command, args.join(" "))
        };

        trimcp_config.servers.insert(
            name.clone(),
            ServerConfig {
                command: command.to_string(),
                args,
                env,
            },
        );

        *entry = serde_json::json!({
            "command": { "path": "trimcp", "args": ["proxy", name] }
        });

        println!("  ✓ {name:<20} {display_cmd}");
        imported += 1;
    }

    Ok(imported)
}

// ── Shared helper ─────────────────────────────────────────────────────────────

/// Try to import a stdio entry (command/args/env format).
/// Returns the server name if imported, None if skipped.
fn import_stdio_entry<'a>(
    name: &'a str,
    entry: &Value,
    trimcp_config: &mut Config,
    _client_name: &str,
) -> Option<&'a str> {
    // Already proxied
    if entry.get("command").and_then(|v| v.as_str()) == Some("trimcp") {
        println!("  ~ {name:<20} already proxied");
        return None;
    }

    // Disabled
    if entry
        .get("disabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        println!("  - {name:<20} skipped (disabled)");
        return None;
    }

    // HTTP-based
    if entry.get("serverUrl").is_some() || entry.get("url").is_some() {
        println!("  - {name:<20} skipped (HTTP server, not supported)");
        return None;
    }

    let command = entry.get("command").and_then(|v| v.as_str())?;

    let args: Vec<String> = entry
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let env: HashMap<String, String> = entry
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let display_cmd = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };

    trimcp_config.servers.insert(
        name.to_string(),
        ServerConfig {
            command: command.to_string(),
            args,
            env,
        },
    );

    println!("  ✓ {name:<20} {display_cmd}");
    Some(name)
}
