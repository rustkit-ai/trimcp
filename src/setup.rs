use crate::config::{Config, ServerConfig};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

struct Client {
    name: &'static str,
    config_path: PathBuf,
    restart_hint: &'static str,
}

fn detect_clients() -> Vec<Client> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let home = PathBuf::from(&home);

    let mut candidates = vec![
        Client {
            name: "Claude Code",
            config_path: home.join(".claude").join("settings.json"),
            restart_hint: "Restart Claude Code to apply changes.",
        },
        Client {
            name: "Cursor",
            config_path: home.join(".cursor").join("mcp.json"),
            restart_hint: "Restart Cursor to apply changes.",
        },
        Client {
            name: "Windsurf",
            config_path: home.join(".codeium").join("windsurf").join("mcp_config.json"),
            restart_hint: "Restart Windsurf to apply changes.",
        },
    ];

    // Claude Desktop — path varies by OS
    #[cfg(target_os = "macos")]
    candidates.push(Client {
        name: "Claude Desktop",
        config_path: home
            .join("Library")
            .join("Application Support")
            .join("Claude")
            .join("claude_desktop_config.json"),
        restart_hint: "Restart Claude Desktop to apply changes.",
    });

    #[cfg(target_os = "linux")]
    candidates.push(Client {
        name: "Claude Desktop",
        config_path: home
            .join(".config")
            .join("Claude")
            .join("claude_desktop_config.json"),
        restart_hint: "Restart Claude Desktop to apply changes.",
    });

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| home.display().to_string());
        candidates.push(Client {
            name: "Claude Desktop",
            config_path: PathBuf::from(appdata)
                .join("Claude")
                .join("claude_desktop_config.json"),
            restart_hint: "Restart Claude Desktop to apply changes.",
        });
    }

    candidates.into_iter().filter(|c| c.config_path.exists()).collect()
}

pub fn run(config_path: &Path) -> Result<()> {
    let clients = detect_clients();

    if clients.is_empty() {
        println!("No MCP clients found (Claude Code, Claude Desktop, Cursor, Windsurf).");
        println!("Add servers manually with: trimcp add <name> -- <command> [args...]");
        return Ok(());
    }

    let mut trimcp_config = Config::load(config_path)?;
    let mut total_imported = 0usize;

    for client in &clients {
        println!("{} ({})", client.name, client.config_path.display());

        let raw = std::fs::read_to_string(&client.config_path)?;
        let mut json: Value = serde_json::from_str(&raw)?;

        let Some(servers) = json
            .get_mut("mcpServers")
            .and_then(|v| v.as_object_mut())
        else {
            println!("  (no mcpServers found)\n");
            continue;
        };

        let mut imported = 0usize;

        for (name, entry) in servers.iter_mut() {
            // Skip servers already proxied by trimcp
            if entry.get("command").and_then(|v| v.as_str()) == Some("trimcp") {
                println!("  ~ {name:<20} already proxied");
                continue;
            }

            // Skip disabled servers
            if entry.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false) {
                println!("  - {name:<20} skipped (disabled)");
                continue;
            }

            // Skip HTTP-based servers (serverUrl instead of command)
            if entry.get("serverUrl").is_some() || entry.get("url").is_some() {
                println!("  - {name:<20} skipped (HTTP server, not supported)");
                continue;
            }

            let Some(command) = entry.get("command").and_then(|v| v.as_str()) else {
                println!("  ? {name:<20} skipped (no command field)");
                continue;
            };

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
                name.clone(),
                ServerConfig {
                    command: command.to_string(),
                    args,
                    env,
                },
            );

            // Replace entry in client config with trimcp proxy
            *entry = serde_json::json!({
                "command": "trimcp",
                "args": ["proxy", name]
            });

            println!("  ✓ {name:<20} {display_cmd}");
            imported += 1;
        }

        if imported > 0 {
            let updated = serde_json::to_string_pretty(&json)?;
            std::fs::write(&client.config_path, updated)?;
            println!("  → {} updated", client.config_path.display());
            total_imported += imported;
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
        for client in &clients {
            println!("→ {}", client.restart_hint);
        }
    } else {
        println!("Nothing to import — all servers already proxied or disabled.");
    }

    Ok(())
}
