use crate::config::{self, Config, ServerStrategy};
use colored::Colorize;
use rustkit_semantic::SemanticIndex;
use std::path::Path;

pub fn run(config_path: &Path) -> anyhow::Result<()> {
    let config = Config::load(config_path)?;

    let knowledge_servers: Vec<(&String, &config::ServerConfig)> = {
        let mut v: Vec<_> = config
            .servers
            .iter()
            .filter(|(_, s)| s.strategy == ServerStrategy::Knowledge)
            .collect();
        v.sort_by_key(|(name, _)| *name);
        v
    };

    let passthrough_count = config.servers.len() - knowledge_servers.len();

    if config.servers.is_empty() {
        println!("No servers configured.");
        println!("Add one with: trimcp add <name> -- <command> [args...]");
        return Ok(());
    }

    let sep = "═".repeat(56);
    println!();
    println!("{}", "trimcp Knowledge Stores".bold());
    println!("{}", sep.dimmed());
    println!(
        "  Threshold : {}   TTL : {} days",
        config.knowledge.threshold,
        config.knowledge.ttl_days
    );
    println!();

    if knowledge_servers.is_empty() {
        println!(
            "  {}",
            "No servers have strategy = \"knowledge\".".yellow()
        );
        println!(
            "  Set strategy = \"knowledge\" in a [servers.<name>] block to enable."
        );
        println!();
        return Ok(());
    }

    let rule = "─".repeat(56);
    println!("{}", rule.dimmed());
    println!(
        "{}",
        format!(
            "  {:<22} {:>8}  {:>7}  {}",
            "Server", "Entries", "Disk", "TTL"
        )
        .bold()
    );
    println!("{}", rule.dimmed());

    for (name, server_cfg) in &knowledge_servers {
        let path = config::knowledge_path(name);
        let ttl_days = server_cfg
            .knowledge_ttl_days
            .unwrap_or(config.knowledge.ttl_days);

        let (entries, disk) = if path.exists() {
            let count = SemanticIndex::entry_count(&path).unwrap_or(0);
            let bytes = std::fs::metadata(&path)
                .map(|m| m.len())
                .unwrap_or(0);
            (count, fmt_bytes(bytes))
        } else {
            (0, "—".to_string())
        };

        println!(
            "  {:<22} {:>8}  {:>7}  {}d",
            name.cyan(),
            entries,
            disk,
            ttl_days,
        );
    }

    println!("{}", rule.dimmed());

    if passthrough_count > 0 {
        println!();
        println!(
            "  {} server{} use strategy = \"passthrough\" (no knowledge store).",
            passthrough_count,
            if passthrough_count == 1 { "" } else { "s" }
        );
    }

    println!();
    Ok(())
}

fn fmt_bytes(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}MB", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}KB", n as f64 / 1_000.0)
    } else {
        format!("{n}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fmt_bytes_bytes() {
        assert_eq!(fmt_bytes(512), "512B");
    }

    #[test]
    fn test_fmt_bytes_kb() {
        assert_eq!(fmt_bytes(4_500), "4KB");
        assert_eq!(fmt_bytes(9_500), "10KB");
    }

    #[test]
    fn test_fmt_bytes_mb() {
        assert_eq!(fmt_bytes(1_200_000), "1.2MB");
    }
}
