#![allow(dead_code)]

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Full proxy configuration loaded from `rustkit-mcp.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub upstream: UpstreamConfig,
    pub compression: CompressionConfig,
    pub metrics: MetricsConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpstreamConfig {
    pub command: String,
    pub args: Vec<String>,
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

// ── Loading ───────────────────────────────────────────────────────────────────

impl Config {
    /// Load config from a TOML file. Falls back to defaults if the file is absent.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Config(format!("cannot read {}: {e}", path.display())))?;

        let config: Self = toml::from_str(&content)
            .map_err(|e| Error::Config(format!("invalid TOML in {}: {e}", path.display())))?;

        config.validate()?;
        Ok(config)
    }

    /// Load from the default path `./rustkit-mcp.toml`.
    pub fn load_default() -> Result<Self> {
        Self::load(Path::new("rustkit-mcp.toml"))
    }

    /// Validate required fields.
    fn validate(&self) -> Result<()> {
        if self.upstream.command.is_empty() {
            return Err(Error::Config("upstream.command is required".to_string()));
        }
        Ok(())
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
    }

    #[test]
    fn test_load_absent_file_returns_default() {
        let cfg = Config::load(Path::new("/nonexistent/rustkit-mcp.toml")).unwrap();
        assert!(cfg.compression.enabled);
    }

    #[test]
    fn test_load_valid_toml() {
        let file = write_toml(
            r#"
[upstream]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[compression]
enabled = true
strip_comments = true
"#,
        );
        let cfg = Config::load(file.path()).unwrap();
        assert_eq!(cfg.upstream.command, "npx");
        assert_eq!(cfg.upstream.args.len(), 3);
        assert!(cfg.compression.strip_comments);
    }

    #[test]
    fn test_load_partial_toml_uses_defaults() {
        let file = write_toml(
            r#"
[upstream]
command = "my-mcp-server"
"#,
        );
        let cfg = Config::load(file.path()).unwrap();
        assert_eq!(cfg.upstream.command, "my-mcp-server");
        assert!(cfg.upstream.args.is_empty());
        assert!(cfg.compression.enabled);
    }

    #[test]
    fn test_validate_fails_with_empty_command() {
        let file = write_toml(
            r#"
[upstream]
command = ""
"#,
        );
        let result = Config::load(file.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("upstream.command"), "got: {msg}");
    }

    #[test]
    fn test_load_invalid_toml_returns_error() {
        let file = write_toml("this is not valid toml ][");
        let result = Config::load(file.path());
        assert!(result.is_err());
    }
}
