#![deny(clippy::all)]

mod cache;
mod compress;
mod config;
mod error;
mod metrics;
mod protocol;
mod proxy;
mod transport;

use clap::{Parser, Subcommand};
use config::{Config, ServerConfig, default_config_path};
use metrics::Metrics;
use proxy::Proxy;
use std::{path::PathBuf, sync::Arc};
use tracing::debug;
use transport::{StdinReader, StdoutWriter};

/// MCP proxy that reduces LLM token costs through compression and caching.
#[derive(Debug, Parser)]
#[command(name = "trimcp", version, about)]
struct Cli {
    /// Path to config file (default: ~/.config/trimcp/config.toml)
    #[arg(short, long, value_name = "FILE", global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Add an MCP server to the config
    Add {
        /// Name to identify this server
        name: String,
        /// Server command and arguments (after --)
        #[arg(last = true, required = true)]
        upstream: Vec<String>,
    },

    /// Remove an MCP server from the config
    Remove {
        /// Server name to remove
        name: String,
    },

    /// List all configured MCP servers
    List,

    /// Run as MCP proxy for a named server (used by LLM clients)
    Proxy {
        /// Server name
        name: String,

        /// Enable real-time metrics output to stderr
        #[arg(short, long)]
        metrics: bool,

        /// Log level (error, warn, info, debug, trace)
        #[arg(long, default_value = "warn", value_name = "LEVEL")]
        log_level: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(default_config_path);

    match cli.command {
        Command::Add { name, upstream } => cmd_add(&config_path, &name, &upstream),
        Command::Remove { name } => cmd_remove(&config_path, &name),
        Command::List => cmd_list(&config_path),
        Command::Proxy {
            name,
            metrics,
            log_level,
        } => cmd_proxy(&config_path, &name, metrics, &log_level).await,
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

fn cmd_add(config_path: &PathBuf, name: &str, upstream: &[String]) -> anyhow::Result<()> {
    let mut config = Config::load(config_path)?;

    if upstream.is_empty() {
        anyhow::bail!("upstream command is required — use: trimcp add {name} -- <command> [args...]");
    }

    config.servers.insert(
        name.to_string(),
        ServerConfig {
            command: upstream[0].clone(),
            args: upstream[1..].to_vec(),
        },
    );

    config.save(config_path)?;
    println!("Added server '{name}': {}", upstream.join(" "));
    Ok(())
}

fn cmd_remove(config_path: &PathBuf, name: &str) -> anyhow::Result<()> {
    let mut config = Config::load(config_path)?;

    if config.servers.remove(name).is_none() {
        anyhow::bail!("server '{name}' not found — run `trimcp list` to see available servers");
    }

    config.save(config_path)?;
    println!("Removed server '{name}'");
    Ok(())
}

fn cmd_list(config_path: &PathBuf) -> anyhow::Result<()> {
    let config = Config::load(config_path)?;

    if config.servers.is_empty() {
        println!("No servers configured.");
        println!("Add one with: trimcp add <name> -- <command> [args...]");
        return Ok(());
    }

    let mut names: Vec<&String> = config.servers.keys().collect();
    names.sort();

    println!("Configured servers ({}):", names.len());
    for name in names {
        let s = &config.servers[name];
        let cmd = if s.args.is_empty() {
            s.command.clone()
        } else {
            format!("{} {}", s.command, s.args.join(" "))
        };
        println!("  {name:<20} {cmd}");
    }

    Ok(())
}

async fn cmd_proxy(
    config_path: &PathBuf,
    name: &str,
    realtime_metrics: bool,
    log_level: &str,
) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .with_writer(std::io::stderr)
        .init();

    let mut config = Config::load(config_path)?;
    let server = config.get_server(name)?.clone();

    if realtime_metrics {
        config.metrics.realtime = true;
    }

    let metrics = Arc::new(Metrics::new());

    debug!(
        command = %server.command,
        args = ?server.args,
        "spawning upstream"
    );

    let mut proxy = {
        let p = Proxy::spawn(&server.command, &server.args, Arc::clone(&metrics))?;
        if config.cache.enabled {
            p.with_cache(config.cache.ttl_secs)
        } else {
            p
        }
    };

    let mut reader = StdinReader::new();
    let mut writer = StdoutWriter::new();

    loop {
        let msg = match reader.read().await? {
            Some(msg) => msg,
            None => break,
        };

        match proxy.forward(&msg).await? {
            Some(response) => {
                if config.metrics.realtime && metrics.tool_calls() > 0 {
                    eprintln!(
                        "[trimcp] saved {} tokens so far ({:.1}%)",
                        metrics.tokens_saved(),
                        metrics.savings_percent()
                    );
                }
                writer.write(&response).await?;
            }
            None => {
                debug!("notification forwarded, no response");
            }
        }
    }

    if config.metrics.enabled {
        metrics.print_summary();
    }

    proxy.shutdown().await?;
    Ok(())
}
