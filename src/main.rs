#![deny(clippy::all)]

mod cache;
mod compress;
mod config;
mod error;
mod metrics;
mod protocol;
mod proxy;
mod transport;

use clap::Parser;
use config::Config;
use metrics::Metrics;
use proxy::Proxy;
use std::{path::PathBuf, sync::Arc};
use tracing::debug;
use transport::{StdinReader, StdoutWriter};

/// MCP proxy to reduce LLM token costs.
#[derive(Debug, Parser)]
#[command(name = "rustkit-mcp", version, about)]
struct Cli {
    /// Path to config file (default: ./rustkit-mcp.toml)
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Enable real-time metrics output to stderr
    #[arg(short, long)]
    metrics: bool,

    /// Log level (error, warn, info, debug, trace)
    #[arg(long, default_value = "warn", value_name = "LEVEL")]
    log_level: String,

    /// Upstream MCP server command and arguments
    #[arg(last = true, required = false)]
    upstream: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(&cli.log_level)
        .with_writer(std::io::stderr)
        .init();

    let config_path = cli
        .config
        .unwrap_or_else(|| PathBuf::from("rustkit-mcp.toml"));

    let mut config = Config::load(&config_path)?;

    // CLI upstream args override config file
    if !cli.upstream.is_empty() {
        config.upstream.command = cli.upstream[0].clone();
        config.upstream.args = cli.upstream[1..].to_vec();
    }

    if config.upstream.command.is_empty() {
        anyhow::bail!(
            "No upstream command provided. Use `rustkit-mcp -- <command> [args...]` or set [upstream] in rustkit-mcp.toml"
        );
    }

    if cli.metrics {
        config.metrics.realtime = true;
    }

    let metrics = Arc::new(Metrics::new());

    debug!(
        command = %config.upstream.command,
        args = ?config.upstream.args,
        "spawning upstream"
    );

    let mut proxy = Proxy::spawn(
        &config.upstream.command,
        &config.upstream.args,
        Arc::clone(&metrics),
    )?;

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
                        "[rustkit-mcp] saved {} tokens so far ({:.1}%)",
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
