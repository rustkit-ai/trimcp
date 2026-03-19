use crate::metrics::Metrics;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-server persistent statistics accumulated across sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerStats {
    pub total_calls: usize,
    pub tokens_in: usize,
    pub tokens_out: usize,
    pub sessions: usize,
    pub cache_hits: usize,
    #[serde(default)]
    pub knowledge_hits: usize,
}

impl ServerStats {
    pub fn tokens_saved(&self) -> usize {
        self.tokens_in.saturating_sub(self.tokens_out)
    }

    pub fn savings_percent(&self) -> f64 {
        if self.tokens_in == 0 {
            return 0.0;
        }
        self.tokens_saved() as f64 / self.tokens_in as f64 * 100.0
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StatsData {
    servers: HashMap<String, ServerStats>,
}

pub struct StatsStore {
    path: PathBuf,
    data: StatsData,
}

impl StatsStore {
    /// Load stats from a JSON file. Returns default (empty) if the file is absent.
    pub fn load(path: &Path) -> Result<Self> {
        let data = if path.exists() {
            let raw = std::fs::read_to_string(path)?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            StatsData::default()
        };
        Ok(Self {
            path: path.to_path_buf(),
            data,
        })
    }

    /// Add session metrics to the named server's cumulative totals.
    #[allow(dead_code)]
    pub fn record(&mut self, server: &str, metrics: &Metrics) {
        let entry = self.data.servers.entry(server.to_string()).or_default();
        entry.total_calls += metrics.tool_calls();
        entry.tokens_in += metrics.tokens_in();
        entry.tokens_out += metrics.tokens_out();
        entry.cache_hits += metrics.cache_hits();
        entry.knowledge_hits += metrics.knowledge_hits();
        entry.sessions += 1;
    }

    /// Increment the session counter for a server (called once per process lifetime).
    pub fn increment_sessions(&mut self, server: &str) {
        self.data
            .servers
            .entry(server.to_string())
            .or_default()
            .sessions += 1;
    }

    /// Add a pre-computed delta (used for mid-session incremental saves).
    pub fn record_delta(
        &mut self,
        server: &str,
        calls: usize,
        tokens_in: usize,
        tokens_out: usize,
        cache_hits: usize,
        knowledge_hits: usize,
    ) {
        let entry = self.data.servers.entry(server.to_string()).or_default();
        entry.total_calls += calls;
        entry.tokens_in += tokens_in;
        entry.tokens_out += tokens_out;
        entry.cache_hits += cache_hits;
        entry.knowledge_hits += knowledge_hits;
    }

    /// Write the stats file to disk, creating parent directories as needed.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(&self.data)?;
        std::fs::write(&self.path, content)?;
        Ok(())
    }

    /// Return all server stats.
    pub fn all(&self) -> &HashMap<String, ServerStats> {
        &self.data.servers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[allow(dead_code)]
    fn make_metrics(calls: usize, tokens_in: usize, tokens_out: usize) -> Arc<Metrics> {
        let m = Arc::new(Metrics::new());
        // Use record() to accumulate the right amounts via estimate_tokens approximation.
        // Instead, we just call record() with strings of the right token size.
        // 1 token ≈ 4 chars, so for `tokens_in` tokens we need tokens_in*4 chars.
        for _ in 0..calls {
            let input = "a".repeat(tokens_in / calls * 4);
            let output = "a".repeat(tokens_out / calls * 4);
            m.record(&input, &output);
        }
        m
    }

    #[test]
    fn test_server_stats_tokens_saved() {
        let s = ServerStats {
            total_calls: 10,
            tokens_in: 1000,
            tokens_out: 600,
            sessions: 2,
            cache_hits: 0,
            knowledge_hits: 0,
        };
        assert_eq!(s.tokens_saved(), 400);
    }

    #[test]
    fn test_server_stats_savings_percent() {
        let s = ServerStats {
            total_calls: 10,
            tokens_in: 1000,
            tokens_out: 600,
            sessions: 2,
            cache_hits: 0,
            knowledge_hits: 0,
        };
        assert!((s.savings_percent() - 40.0).abs() < 0.001);
    }

    #[test]
    fn test_server_stats_savings_percent_zero_tokens_in() {
        let s = ServerStats::default();
        assert_eq!(s.savings_percent(), 0.0);
    }

    #[test]
    fn test_stats_store_load_nonexistent_returns_empty() {
        let store = StatsStore::load(Path::new("/nonexistent/path/stats.json")).unwrap();
        assert!(store.all().is_empty());
    }

    #[test]
    fn test_stats_store_record_accumulates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stats.json");
        let mut store = StatsStore::load(&path).unwrap();

        let m = Metrics::new();
        m.record("a".repeat(400).as_str(), "a".repeat(200).as_str());
        store.record("my-server", &m);

        let stats = store.all().get("my-server").unwrap();
        assert_eq!(stats.sessions, 1);
        assert_eq!(stats.total_calls, 1);
        assert!(stats.tokens_in > 0);
    }

    #[test]
    fn test_stats_store_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stats.json");

        {
            let mut store = StatsStore::load(&path).unwrap();
            let m = Metrics::new();
            m.record("a".repeat(400).as_str(), "a".repeat(200).as_str());
            store.record("server-a", &m);
            store.save().unwrap();
        }

        let store2 = StatsStore::load(&path).unwrap();
        let stats = store2.all().get("server-a").unwrap();
        assert_eq!(stats.sessions, 1);
        assert_eq!(stats.total_calls, 1);
    }

    #[test]
    fn test_stats_store_record_multiple_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stats.json");

        for i in 0..3u32 {
            let mut store = StatsStore::load(&path).unwrap();
            let m = Metrics::new();
            m.record(
                "a".repeat(400 * (i as usize + 1)).as_str(),
                "a".repeat(200).as_str(),
            );
            store.record("server-b", &m);
            store.save().unwrap();
        }

        let store = StatsStore::load(&path).unwrap();
        let stats = store.all().get("server-b").unwrap();
        assert_eq!(stats.sessions, 3);
        assert_eq!(stats.total_calls, 3);
    }
}
