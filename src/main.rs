#![deny(clippy::all)]

mod cache;
mod clients;
mod cmd_knowledge;
mod cmd_stats;
#[cfg(feature = "semtree")]
mod code_context;
mod compress;
mod config;
mod error;
mod knowledge;
mod metrics;
mod protocol;
mod proxy;
mod setup;
mod stats_store;
mod status;
mod transport;

use clap::{Parser, Subcommand};
#[cfg(feature = "semtree")]
use code_context::CodeContext;
use colored::Colorize;
use config::{Config, ServerConfig, ServerStrategy, default_config_path};
use knowledge::KnowledgeStore;
use metrics::Metrics;
use proxy::Proxy;
use stats_store::StatsStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info, warn};
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

        /// Write logs to a file instead of (or in addition to) stderr
        #[arg(long, value_name = "FILE")]
        log_file: Option<PathBuf>,
    },

    /// Auto-detect MCP clients and set up trimcp as proxy
    Setup,

    /// Show MCP server status across all clients
    Status,

    /// Show token savings statistics
    Stats {
        /// Server name (optional, shows all if omitted)
        name: Option<String>,
    },

    /// Show knowledge store status (entries, disk usage, TTL)
    Knowledge,

    #[cfg(feature = "semtree")]
    /// Index a codebase with semtree for a named server
    SemtreeIndex {
        /// Server name (must be configured with `semtree_codebase` set, or pass `--path`)
        name: String,
        /// Path to the codebase to index (overrides `semtree_codebase` in config)
        #[arg(short, long, value_name = "DIR")]
        path: Option<PathBuf>,
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
            log_file,
        } => {
            cmd_proxy(
                &config_path,
                &name,
                metrics,
                &log_level,
                log_file.as_deref(),
            )
            .await
        }
        Command::Setup => setup::run(&config_path),
        Command::Status => status::run(&config_path),
        Command::Stats { name } => cmd_stats::run(&config_path, name.as_deref()),
        Command::Knowledge => cmd_knowledge::run(&config_path),
        #[cfg(feature = "semtree")]
        Command::SemtreeIndex { name, path } => {
            cmd_semtree_index(&config_path, &name, path.as_deref()).await
        }
    }
}

// ── Commands ──────────────────────────────────────────────────────────────────

fn cmd_add(config_path: &Path, name: &str, upstream: &[String]) -> anyhow::Result<()> {
    let mut config = Config::load(config_path)?;

    if upstream.is_empty() {
        anyhow::bail!(
            "upstream command is required — use: trimcp add {name} -- <command> [args...]"
        );
    }

    config.servers.insert(
        name.to_string(),
        ServerConfig {
            command: upstream[0].clone(),
            args: upstream[1..].to_vec(),
            env: std::collections::HashMap::new(),
            ..Default::default()
        },
    );

    config.save(config_path)?;
    println!(
        "{} '{}': {}",
        "Added server".green().bold(),
        name,
        upstream.join(" ")
    );
    Ok(())
}

fn cmd_remove(config_path: &Path, name: &str) -> anyhow::Result<()> {
    let mut config = Config::load(config_path)?;

    if config.servers.remove(name).is_none() {
        anyhow::bail!("server '{name}' not found — run `trimcp list` to see available servers");
    }

    config.save(config_path)?;
    println!("{} '{name}'", "Removed server".green().bold());
    Ok(())
}

fn cmd_list(config_path: &Path) -> anyhow::Result<()> {
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
        println!("  {:<20} {}", name.cyan(), cmd);
    }

    Ok(())
}

async fn cmd_proxy(
    config_path: &Path,
    name: &str,
    realtime_metrics: bool,
    log_level: &str,
    log_file: Option<&Path>,
) -> anyhow::Result<()> {
    if let Some(path) = log_file {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        tracing_subscriber::fmt()
            .with_env_filter(log_level)
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(log_level)
            .with_writer(std::io::stderr)
            .init();
    }

    let mut config = Config::load(config_path)?;
    let server = config.get_server(name)?.clone();

    if realtime_metrics {
        config.metrics.realtime = true;
    }

    let metrics = Arc::new(Metrics::new());

    info!(
        server = %name,
        command = %server.command,
        args = ?server.args,
        metrics_enabled = config.metrics.enabled,
        cache_enabled = config.cache.enabled,
        stats_path = %config::stats_path().display(),
        "trimcp proxy starting"
    );

    let mut proxy = {
        let mut p = Proxy::spawn(
            &server.command,
            &server.args,
            &server.env,
            Arc::clone(&metrics),
        )?;
        if config.cache.enabled {
            p = p.with_cache(config.cache.ttl_secs, config::cache_path(name));
        }
        if server.strategy == ServerStrategy::Knowledge {
            let ttl_days = server
                .knowledge_ttl_days
                .unwrap_or(config.knowledge.ttl_days);
            match KnowledgeStore::open(
                &config::knowledge_path(name),
                config.knowledge.threshold,
                ttl_days,
            ) {
                Ok(store) => {
                    info!(
                        server = %name,
                        ttl_days,
                        threshold = config.knowledge.threshold,
                        "knowledge store enabled"
                    );
                    p = p.with_knowledge_store(store);
                }
                Err(e) => {
                    warn!(err = %e, "knowledge store init failed, running without it");
                }
            }
        }
        #[cfg(feature = "semtree")]
        if server.semtree_codebase.is_some() {
            let top_k = server.semtree_top_k.unwrap_or(config.semtree.top_k);
            let index_path = config::semtree_index_path(name);
            match CodeContext::load(&index_path, top_k) {
                Ok(ctx) => {
                    info!(
                        server = %name,
                        chunks = ctx.len(),
                        top_k,
                        "semtree code context enabled"
                    );
                    p = p.with_code_context(ctx);
                }
                Err(e) => {
                    warn!(
                        err = %e,
                        "semtree index not ready — run `trimcp semtree-index {name}` to build it"
                    );
                }
            }
        }
        p
    };

    let mut reader = StdinReader::new();
    let mut writer = StdoutWriter::new();

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    // Track what has already been persisted to compute deltas incrementally.
    let mut saved_calls: usize = 0;
    let mut saved_tokens_in: usize = 0;
    let mut saved_tokens_out: usize = 0;
    let mut saved_cache_hits: usize = 0;
    let mut saved_knowledge_hits: usize = 0;

    loop {
        tokio::select! {
            msg = reader.read() => {
                let msg = match msg? {
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
                        // Persist delta after each tool call so `trimcp stats`
                        // reflects live data without waiting for process exit.
                        if config.metrics.enabled {
                            let cur_calls = metrics.tool_calls();
                            let cur_cache_hits = metrics.cache_hits();
                            debug!(
                                tool_calls = cur_calls,
                                cache_hits = cur_cache_hits,
                                saved_calls,
                                "checking if delta save needed"
                            );
                            if cur_calls > saved_calls {
                                let stats_path = config::stats_path();
                                match StatsStore::load(&stats_path) {
                                    Err(e) => error!(path = %stats_path.display(), err = %e, "failed to load stats store"),
                                    Ok(mut store) => {
                                        let delta_calls = cur_calls - saved_calls;
                                        let delta_in = metrics.tokens_in() - saved_tokens_in;
                                        let delta_out = metrics.tokens_out() - saved_tokens_out;
                                        let delta_hits = metrics.cache_hits() - saved_cache_hits;
                                        let delta_knowledge = metrics.knowledge_hits() - saved_knowledge_hits;
                                        info!(
                                            delta_calls,
                                            delta_tokens_in = delta_in,
                                            delta_tokens_out = delta_out,
                                            delta_cache_hits = delta_hits,
                                            delta_knowledge_hits = delta_knowledge,
                                            "saving stats delta"
                                        );
                                        store.record_delta(name, delta_calls, delta_in, delta_out, delta_hits, delta_knowledge);
                                        match store.save() {
                                            Ok(()) => {
                                                info!(path = %stats_path.display(), "stats saved");
                                                saved_calls = cur_calls;
                                                saved_tokens_in = metrics.tokens_in();
                                                saved_tokens_out = metrics.tokens_out();
                                                saved_cache_hits = metrics.cache_hits();
                                                saved_knowledge_hits = metrics.knowledge_hits();
                                            }
                                            Err(e) => error!(path = %stats_path.display(), err = %e, "failed to save stats"),
                                        }
                                    }
                                }
                            } else if cur_cache_hits > saved_cache_hits {
                                warn!(
                                    cur_cache_hits,
                                    saved_cache_hits,
                                    "cache hits occurred but tool_calls not incremented — cache hits not counted in stats"
                                );
                            }
                        }
                    }
                    None => {
                        debug!("notification forwarded, no response");
                    }
                }
            }
            _ = sigterm.recv() => {
                debug!("received SIGTERM, shutting down gracefully");
                break;
            }
        }
    }

    if config.metrics.enabled {
        metrics.print_summary();
        // Persist any remaining unsaved delta (e.g. calls that arrived between
        // the last incremental save and process exit via EOF / SIGTERM).
        let remaining = metrics.tool_calls() - saved_calls;
        if remaining > 0 {
            let stats_path = config::stats_path();
            if let Ok(mut store) = StatsStore::load(&stats_path) {
                store.record_delta(
                    name,
                    remaining,
                    metrics.tokens_in() - saved_tokens_in,
                    metrics.tokens_out() - saved_tokens_out,
                    metrics.cache_hits() - saved_cache_hits,
                    metrics.knowledge_hits() - saved_knowledge_hits,
                );
                let _ = store.save();
            }
        }
        // Increment the session counter once per process lifetime.
        if metrics.tool_calls() > 0 {
            let stats_path = config::stats_path();
            if let Ok(mut store) = StatsStore::load(&stats_path) {
                store.increment_sessions(name);
                let _ = store.save();
            }
        }
    }

    proxy.shutdown().await?;

    Ok(())
}

#[cfg(feature = "semtree")]
async fn cmd_semtree_index(
    config_path: &Path,
    name: &str,
    path_override: Option<&Path>,
) -> anyhow::Result<()> {
    let config = Config::load(config_path)?;
    let server = config.get_server(name)?;

    let codebase_dir = match path_override {
        Some(p) => p.to_path_buf(),
        None => server.semtree_codebase.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "no codebase path for server '{name}' — \
                     set `semtree_codebase` in config or pass --path <dir>"
            )
        })?,
    };

    let top_k = server.semtree_top_k.unwrap_or(config.semtree.top_k);
    let index_path = config::semtree_index_path(name);

    println!(
        "{} '{}': {}",
        "Indexing".green().bold(),
        name,
        codebase_dir.display()
    );

    let ctx = CodeContext::build(&codebase_dir, &index_path, top_k).await?;

    println!(
        "{} {} chunks → {}",
        "Indexed".green().bold(),
        ctx.len(),
        index_path.display()
    );

    Ok(())
}
