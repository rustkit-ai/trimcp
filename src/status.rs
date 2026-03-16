use crate::clients::{self, ServerStatus};
use std::collections::HashMap;
use std::path::Path;

pub fn run(_config_path: &Path) -> anyhow::Result<()> {
    let detected = clients::detect_all();

    println!("Scanning MCP clients...");
    println!();

    // Print per-client summary
    for client in &detected {
        let count = client.servers.len();
        if count == 0 {
            println!("  {:<20} no mcpServers configured", client.def.name);
        } else {
            println!(
                "  {:<20} {} server{}",
                client.def.name,
                count,
                if count == 1 { "" } else { "s" }
            );
        }
    }

    // Build map: server_name -> Vec<(client_name, ServerStatus)>
    let mut server_map: HashMap<String, Vec<(&'static str, ServerStatus)>> = HashMap::new();
    for client in &detected {
        for server in &client.servers {
            server_map
                .entry(server.name.clone())
                .or_default()
                .push((client.def.name, server.status.clone()));
        }
    }

    if server_map.is_empty() {
        return Ok(());
    }

    println!();
    println!("Server status:");
    println!();
    println!("  {:<20} {:<12} CLIENTS", "NAME", "STATUS");

    // Sort entries: proxied first, then direct, disabled, http
    let mut entries: Vec<(String, Vec<(&'static str, ServerStatus)>)> =
        server_map.into_iter().collect();
    entries.sort_by(|a, b| {
        let rank = |entries: &Vec<(&'static str, ServerStatus)>| {
            // Use the "best" status across all clients for ordering
            entries
                .iter()
                .map(|(_, s)| status_rank(s))
                .min()
                .unwrap_or(99)
        };
        rank(&a.1).cmp(&rank(&b.1)).then_with(|| a.0.cmp(&b.0))
    });

    for (name, client_entries) in &entries {
        // Determine the combined status label (use first entry's status per client group)
        // For display, pick the status of the first occurrence
        let (_, status) = &client_entries[0];
        let status_label = format_status(status);
        let client_names: Vec<&str> = client_entries.iter().map(|(c, _)| *c).collect();
        println!(
            "  {:<20} {:<12} {}",
            name,
            status_label,
            client_names.join(", ")
        );
    }

    Ok(())
}

fn status_rank(s: &ServerStatus) -> u8 {
    match s {
        ServerStatus::Proxied => 0,
        ServerStatus::Direct => 1,
        ServerStatus::Disabled => 2,
        ServerStatus::Http => 3,
    }
}

fn format_status(s: &ServerStatus) -> &'static str {
    match s {
        ServerStatus::Proxied => "\u{2713} proxied", // ✓ proxied
        ServerStatus::Direct => "\u{2717} direct",   // ✗ direct
        ServerStatus::Disabled => "- disabled",
        ServerStatus::Http => "~ http",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_status_proxied() {
        assert_eq!(format_status(&ServerStatus::Proxied), "✓ proxied");
    }

    #[test]
    fn test_format_status_direct() {
        assert_eq!(format_status(&ServerStatus::Direct), "✗ direct");
    }

    #[test]
    fn test_format_status_disabled() {
        assert_eq!(format_status(&ServerStatus::Disabled), "- disabled");
    }

    #[test]
    fn test_format_status_http() {
        assert_eq!(format_status(&ServerStatus::Http), "~ http");
    }

    #[test]
    fn test_status_rank_ordering() {
        assert!(status_rank(&ServerStatus::Proxied) < status_rank(&ServerStatus::Direct));
        assert!(status_rank(&ServerStatus::Direct) < status_rank(&ServerStatus::Disabled));
        assert!(status_rank(&ServerStatus::Disabled) < status_rank(&ServerStatus::Http));
    }
}
