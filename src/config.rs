use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Full proxy configuration loaded from `~/.config/trimcp/config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub servers: HashMap<String, ServerConfig>,
    pub compression: CompressionConfig,
    pub metrics: MetricsConfig,
    pub cache: CacheConfig,
    pub knowledge: KnowledgeConfig,
    pub semtree: SemtreeConfig,
}

/// Global defaults for the semantic knowledge store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KnowledgeConfig {
    /// Minimum cosine-similarity score `[0.0, 1.0]` to accept a semantic hit.
    pub threshold: f32,
    /// How many days to keep stored responses before they expire.
    pub ttl_days: u64,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            threshold: 0.82,
            ttl_days: 7,
        }
    }
}

/// Global defaults for the semtree code-context injection layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SemtreeConfig {
    /// Number of code chunks to inject into each tool response.
    pub top_k: usize,
}

impl Default for SemtreeConfig {
    fn default() -> Self {
        Self { top_k: 3 }
    }
}

/// How a server's tool-call responses are stored and retrieved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerStrategy {
    /// Pass every call upstream (no knowledge store). Default.
    #[default]
    Passthrough,
    /// Index responses semantically; similar future queries get a local hit.
    Knowledge,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    /// Caching strategy for this server (default: passthrough).
    pub strategy: ServerStrategy,
    /// Override the global `knowledge.ttl_days` for this server.
    pub knowledge_ttl_days: Option<u64>,
    /// Path to the codebase to index with semtree. When set, the proxy injects
    /// relevant code context into every `tools/call` response going back to the LLM.
    pub semtree_codebase: Option<PathBuf>,
    /// Override the global `semtree.top_k` for this server.
    pub semtree_top_k: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressionConfig {
    pub enabled: bool,
    pub strip_ansi: bool,
    pub compact_json: bool,
    pub strip_comments: bool,
    pub dedup: bool,
    pub minify: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub realtime: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl_secs: u64,
}

// ── Defaults ──────────────────────────────────────────────────────────────────

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            strip_ansi: true,
            compact_json: true,
            strip_comments: false,
            dedup: true,
            minify: true,
        }
    }
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            realtime: false,
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl_secs: 300,
        }
    }
}

// ── Path ──────────────────────────────────────────────────────────────────────

/// Default config file path: `~/.config/trimcp/config.toml`.
pub fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("trimcp")
        .join("config.toml")
}

/// Persistent stats file path: `~/.config/trimcp/stats.json`.
pub fn stats_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("trimcp")
        .join("stats.json")
}

/// Persistent knowledge store path: `~/.config/trimcp/knowledge/<server>.db`.
pub fn knowledge_path(server: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("trimcp")
        .join("knowledge")
        .join(format!("{server}.db"))
}

/// Persistent cache file path: `~/.config/trimcp/cache/<server>.json`.
pub fn cache_path(server: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("trimcp")
        .join("cache")
        .join(format!("{server}.json"))
}

/// Persistent semtree index directory: `~/.config/trimcp/semtree/<server>/`.
pub fn semtree_index_path(server: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("trimcp")
        .join("semtree")
        .join(server)
}

// ── Loading / Saving ──────────────────────────────────────────────────────────

impl Config {
    /// Load config from a TOML file. Falls back to defaults if the file is absent.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("cannot read {}: {e}", path.display())))?;

        toml::from_str(&content)
            .map_err(|e| Error::Config(format!("invalid TOML in {}: {e}", path.display())))
    }

    /// Persist config to a TOML file, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::Config(format!(
                    "cannot create config dir {}: {e}",
                    parent.display()
                ))
            })?;
        }

        let content = toml::to_string_pretty(self)
            .map_err(|e| Error::Config(format!("cannot serialize config: {e}")))?;

        std::fs::write(path, content)
            .map_err(|e| Error::Config(format!("cannot write {}: {e}", path.display())))
    }

    /// Get a server by name or return a clear error.
    pub fn get_server(&self, name: &str) -> Result<&ServerConfig> {
        self.servers.get(name).ok_or_else(|| {
            Error::Config(format!(
                "server '{name}' not found — run `trimcp list` to see available servers"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_toml(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    #[test]
    fn test_default_config_has_sensible_values() {
        let cfg = Config::default();
        assert!(cfg.compression.enabled);
        assert!(cfg.compression.strip_ansi);
        assert!(cfg.compression.dedup);
        assert!(!cfg.compression.strip_comments);
        assert!(cfg.metrics.enabled);
        assert!(!cfg.metrics.realtime);
        assert!(cfg.cache.enabled);
        assert_eq!(cfg.cache.ttl_secs, 300);
    }

    #[test]
    fn test_load_absent_file_returns_default() {
        let cfg = Config::load(Path::new("/nonexistent/trimcp.toml")).unwrap();
        assert!(cfg.compression.enabled);
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn test_load_valid_toml_with_servers() {
        let file = write_toml(
            r#"
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[compression]
enabled = true
strip_comments = true
"#,
        );
        let cfg = Config::load(file.path()).unwrap();
        assert_eq!(cfg.servers.len(), 2);
        let fs = cfg.servers.get("filesystem").unwrap();
        assert_eq!(fs.command, "npx");
        assert_eq!(fs.args.len(), 3);
        assert!(cfg.compression.strip_comments);
    }

    #[test]
    fn test_get_server_found() {
        let file = write_toml(
            r#"
[servers.myserver]
command = "my-mcp"
args = []
"#,
        );
        let cfg = Config::load(file.path()).unwrap();
        let s = cfg.get_server("myserver").unwrap();
        assert_eq!(s.command, "my-mcp");
    }

    #[test]
    fn test_get_server_not_found_returns_error() {
        let cfg = Config::default();
        let result = cfg.get_server("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nonexistent"));
    }

    #[test]
    fn test_save_and_reload_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut cfg = Config::default();
        cfg.servers.insert(
            "test".to_string(),
            ServerConfig {
                command: "my-cmd".to_string(),
                args: vec!["--flag".to_string()],
                env: HashMap::new(),
                ..Default::default()
            },
        );
        cfg.save(&path).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.servers.len(), 1);
        let s = loaded.servers.get("test").unwrap();
        assert_eq!(s.command, "my-cmd");
        assert_eq!(s.args, vec!["--flag"]);
    }

    #[test]
    fn test_load_invalid_toml_returns_error() {
        let file = write_toml("this is not valid toml ][");
        let result = Config::load(file.path());
        assert!(result.is_err());
    }
}
