use crate::protocol::JsonRpcResponse;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

struct CacheEntry {
    response: JsonRpcResponse,
    expires_at: Instant,
}

/// TTL-based in-memory cache for tool call results.
pub struct Cache {
    entries: HashMap<u64, CacheEntry>,
    ttl: Duration,
}

impl Cache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: HashMap::new(),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Look up a cached response. Returns `None` if missing or expired.
    pub fn get(&self, key: u64) -> Option<&JsonRpcResponse> {
        let entry = self.entries.get(&key)?;
        if entry.expires_at > Instant::now() {
            Some(&entry.response)
        } else {
            None
        }
    }

    /// Insert a response into the cache.
    pub fn insert(&mut self, key: u64, response: JsonRpcResponse) {
        self.entries.insert(
            key,
            CacheEntry {
                response,
                expires_at: Instant::now() + self.ttl,
            },
        );
    }

    /// Remove all expired entries.
    pub fn evict_expired(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, e| e.expires_at > now);
    }

    /// Number of live (non-expired) entries.
    pub fn len(&self) -> usize {
        let now = Instant::now();
        self.entries.values().filter(|e| e.expires_at > now).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Compute a stable cache key from the tool name and its JSON arguments.
pub fn make_cache_key(tool_name: &str, args: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    tool_name.hash(&mut hasher);
    args.to_string().hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{JsonRpcResponse, RequestId};
    use serde_json::json;
    use std::thread;

    fn make_response(id: i64) -> JsonRpcResponse {
        JsonRpcResponse::ok(RequestId::Number(id), json!({"content": []}))
    }

    #[test]
    fn test_insert_and_get_returns_response() {
        let mut cache = Cache::new(60);
        let resp = make_response(1);
        cache.insert(42, resp);
        assert!(cache.get(42).is_some());
    }

    #[test]
    fn test_get_missing_key_returns_none() {
        let cache = Cache::new(60);
        assert!(cache.get(999).is_none());
    }

    #[test]
    fn test_expired_entry_returns_none() {
        let mut cache = Cache::new(0); // 0s TTL — expires immediately
        cache.insert(1, make_response(1));
        thread::sleep(std::time::Duration::from_millis(10));
        assert!(cache.get(1).is_none());
    }

    #[test]
    fn test_len_counts_live_entries() {
        let mut cache = Cache::new(60);
        cache.insert(1, make_response(1));
        cache.insert(2, make_response(2));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_evict_expired_removes_stale_entries() {
        let mut cache = Cache::new(0);
        cache.insert(1, make_response(1));
        thread::sleep(std::time::Duration::from_millis(10));
        cache.evict_expired();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_make_cache_key_same_input_same_key() {
        let a = make_cache_key("tools/call", &json!({"name": "read_file", "path": "/tmp"}));
        let b = make_cache_key("tools/call", &json!({"name": "read_file", "path": "/tmp"}));
        assert_eq!(a, b);
    }

    #[test]
    fn test_make_cache_key_different_args_different_key() {
        let a = make_cache_key("tools/call", &json!({"path": "/tmp/a"}));
        let b = make_cache_key("tools/call", &json!({"path": "/tmp/b"}));
        assert_ne!(a, b);
    }

    #[test]
    fn test_make_cache_key_different_tool_different_key() {
        let a = make_cache_key("read_file", &json!({}));
        let b = make_cache_key("write_file", &json!({}));
        assert_ne!(a, b);
    }
}
