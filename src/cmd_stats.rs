use crate::config;
use crate::stats_store::{ServerStats, StatsStore};
use colored::Colorize;
use std::path::Path;

const BAR_WIDTH: usize = 20;

pub fn run(_config_path: &Path, server: Option<&str>) -> anyhow::Result<()> {
    let stats_path = config::stats_path();
    let store = StatsStore::load(&stats_path)?;
    let all = store.all();

    if all.is_empty() {
        println!("No stats recorded yet.");
        println!("Run `trimcp proxy <name>` to start accumulating statistics.");
        return Ok(());
    }

    if let Some(name) = server {
        match all.get(name) {
            None => anyhow::bail!("no stats found for server '{name}'"),
            Some(stats) => print_single(name, stats),
        }
        return Ok(());
    }

    print_all(all)
}

// ── Single-server view ────────────────────────────────────────────────────────

fn print_single(name: &str, s: &ServerStats) {
    let sep = "═".repeat(52);
    println!();
    println!("{}", format!("trimcp · {name}").bold());
    println!("{}", sep.dimmed());
    println!();
    println!("  {:<14} {}", "Sessions:".bold(), s.sessions);
    println!("  {:<14} {}", "Tool calls:".bold(), fmt_num(s.total_calls));
    println!("  {:<14} {}", "Cache hits:".bold(), fmt_num(s.cache_hits));
    println!(
        "  {:<14} {}",
        "Knowledge hits:".bold(),
        fmt_num(s.knowledge_hits)
    );
    println!("  {:<14} {}", "Tokens in:".bold(), fmt_k(s.tokens_in));
    println!("  {:<14} {}", "Tokens out:".bold(), fmt_k(s.tokens_out));
    println!(
        "  {:<14} {}  ({})",
        "Tokens saved:".bold(),
        fmt_k(s.tokens_saved()),
        colorize_pct(s.savings_percent())
    );
    println!();
    println!(
        "  {:<14} {}  {}",
        "Efficiency:".bold(),
        progress_bar(s.savings_percent(), BAR_WIDTH),
        colorize_pct(s.savings_percent())
    );
    println!();
}

// ── All-servers view ──────────────────────────────────────────────────────────

fn print_all(all: &std::collections::HashMap<String, ServerStats>) -> anyhow::Result<()> {
    // Totals
    let total_calls: usize = all.values().map(|s| s.total_calls).sum();
    let total_cache_hits: usize = all.values().map(|s| s.cache_hits).sum();
    let total_knowledge_hits: usize = all.values().map(|s| s.knowledge_hits).sum();
    let total_sessions: usize = all.values().map(|s| s.sessions).sum();
    let total_in: usize = all.values().map(|s| s.tokens_in).sum();
    let total_out: usize = all.values().map(|s| s.tokens_out).sum();
    let total_saved = total_in.saturating_sub(total_out);
    let total_pct = if total_in == 0 {
        0.0
    } else {
        total_saved as f64 / total_in as f64 * 100.0
    };

    let sep = "═".repeat(60);
    println!();
    println!("{}", "trimcp Token Savings".bold());
    println!("{}", sep.dimmed());
    println!();
    println!("  {:<16} {}", "Tool calls:".bold(), fmt_num(total_calls));
    println!(
        "  {:<16} {}",
        "Cache hits:".bold(),
        fmt_num(total_cache_hits)
    );
    println!(
        "  {:<16} {}",
        "Knowledge hits:".bold(),
        fmt_num(total_knowledge_hits)
    );
    println!("  {:<16} {}", "Tokens in:".bold(), fmt_k(total_in));
    println!("  {:<16} {}", "Tokens out:".bold(), fmt_k(total_out));
    println!(
        "  {:<16} {}  ({})",
        "Tokens saved:".bold(),
        fmt_k(total_saved),
        colorize_pct(total_pct)
    );
    println!("  {:<16} {}", "Sessions:".bold(), total_sessions);
    println!();
    println!(
        "  {:<16} {}  {}",
        "Efficiency:".bold(),
        progress_bar(total_pct, BAR_WIDTH),
        colorize_pct(total_pct)
    );

    // Table
    println!();
    println!("{}", "By Server".bold());

    let rule = "─".repeat(92);
    println!("{}", rule.dimmed());
    println!(
        "{}",
        format!(
            "  {:<3} {:<22} {:>6}  {:>5}  {:>5}  {:>7}  {:>5}  {:>8}  {}",
            "#", "Server", "Calls", "Cache", "Know.", "Saved", "%", "Sessions", "Impact"
        )
        .bold()
    );
    println!("{}", rule.dimmed());

    // Sort by tokens saved descending
    let mut entries: Vec<(&String, &ServerStats)> = all.iter().collect();
    entries.sort_by(|a, b| b.1.tokens_saved().cmp(&a.1.tokens_saved()));

    let max_saved = entries
        .first()
        .map(|(_, s)| s.tokens_saved())
        .unwrap_or(1)
        .max(1);

    for (rank, (name, stats)) in entries.iter().enumerate() {
        let impact = impact_bar(stats.tokens_saved(), max_saved, 10);
        println!(
            "  {:>2}. {:<22} {:>6}  {:>5}  {:>5}  {:>7}  {:>5}  {:>8}  {}",
            rank + 1,
            truncate(name, 22),
            fmt_num(stats.total_calls),
            fmt_num(stats.cache_hits),
            fmt_num(stats.knowledge_hits),
            fmt_k(stats.tokens_saved()),
            colorize_pct(stats.savings_percent()),
            stats.sessions,
            impact,
        );
    }

    println!("{}", rule.dimmed());
    println!(
        "  {:<26} {:>6}  {:>5}  {:>5}  {:>7}  {}",
        "Total".bold(),
        fmt_num(total_calls).bold().to_string(),
        fmt_num(total_cache_hits).bold().to_string(),
        fmt_num(total_knowledge_hits).bold().to_string(),
        fmt_k(total_saved).bold().to_string(),
        colorize_pct(total_pct),
    );
    println!();

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn colorize_pct(pct: f64) -> String {
    let s = format!("{:.1}%", pct);
    if pct >= 50.0 {
        s.green().bold().to_string()
    } else if pct >= 20.0 {
        s.yellow().to_string()
    } else {
        s.red().to_string()
    }
}

fn progress_bar(pct: f64, width: usize) -> String {
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));
    if pct >= 50.0 {
        bar.green().to_string()
    } else if pct >= 20.0 {
        bar.yellow().to_string()
    } else {
        bar.red().to_string()
    }
}

fn impact_bar(value: usize, max: usize, width: usize) -> String {
    let filled = ((value as f64 / max as f64) * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
        .dimmed()
        .to_string()
}

/// Format a number with K/M suffix.
fn fmt_k(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result.chars().rev().collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fmt_k_small() {
        assert_eq!(fmt_k(42), "42");
    }

    #[test]
    fn test_fmt_k_thousands() {
        assert_eq!(fmt_k(1_500), "1.5K");
        assert_eq!(fmt_k(144_200), "144.2K");
    }

    #[test]
    fn test_fmt_k_millions() {
        assert_eq!(fmt_k(1_234_567), "1.2M");
    }

    #[test]
    fn test_fmt_num_thousands() {
        assert_eq!(fmt_num(1_000), "1,000");
        assert_eq!(fmt_num(12_345), "12,345");
        assert_eq!(fmt_num(1_234_567), "1,234,567");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world foo", 10), "hello wor…");
    }

    #[test]
    fn test_progress_bar_length() {
        // Strip ANSI before measuring length
        let bar = progress_bar(66.7, 20);
        // bar contains ANSI codes; count only █ and ░
        let plain: String = bar.chars().filter(|c| *c == '█' || *c == '░').collect();
        assert_eq!(plain.chars().count(), 20);
    }
}
