use crate::clients::{self, ClientDef, Format};
use crate::config::{Config, ServerConfig};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

// ── Shim types for local use ──────────────────────────────────────────────────

struct Client {
    name: &'static str,
    config_path: std::path::PathBuf,
    format: Format,
    restart_hint: &'static str,
}

impl From<ClientDef> for Client {
    fn from(def: ClientDef) -> Self {
        Self {
            name: def.name,
            config_path: def.config_path,
            format: def.format,
            restart_hint: def.restart_hint,
        }
    }
}

fn detect_clients() -> Vec<Client> {
    clients::all_client_defs()
        .into_iter()
        .filter(|c| c.config_path.exists())
        .map(Client::from)
        .collect()
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run(config_path: &Path) -> Result<()> {
    let clients = detect_clients();

    if clients.is_empty() {
        println!("No MCP clients found.");
        println!(
            "Supported: Claude Code, Claude Desktop, Cursor, Windsurf, VS Code, Zed, Continue.dev"
        );
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
        if raw.trim().is_empty() {
            println!("  (empty file, skipping)\n");
            continue;
        }
        let mut json: Value = serde_json::from_str(&raw)?;

        let imported = match client.format {
            Format::Mcp => process_mcp_servers(&mut json, &mut trimcp_config, client.name)?,
            Format::VsCode => process_vscode_servers(&mut json, &mut trimcp_config, client.name)?,
            Format::ZedContext => process_zed_servers(&mut json, &mut trimcp_config, client.name)?,
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
    let Some(servers) = json.get_mut("mcpServers").and_then(|v| v.as_object_mut()) else {
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
    let Some(servers) = json.get_mut("servers").and_then(|v| v.as_object_mut()) else {
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
                ..Default::default()
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
            ..Default::default()
        },
    );

    println!("  ✓ {name:<20} {display_cmd}");
    Some(name)
}
