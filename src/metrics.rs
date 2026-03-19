#![allow(dead_code)]

use crate::compress::estimate_tokens;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Tracks token savings across a proxy session.
pub struct Metrics {
    tool_calls: AtomicUsize,
    tokens_in: AtomicUsize,
    tokens_out: AtomicUsize,
    cache_hits: AtomicUsize,
    knowledge_hits: AtomicUsize,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            tool_calls: AtomicUsize::new(0),
            tokens_in: AtomicUsize::new(0),
            tokens_out: AtomicUsize::new(0),
            cache_hits: AtomicUsize::new(0),
            knowledge_hits: AtomicUsize::new(0),
        }
    }

    /// Record a compression event.
    pub fn record(&self, original: &str, compressed: &str) {
        self.tool_calls.fetch_add(1, Ordering::Relaxed);
        self.tokens_in
            .fetch_add(estimate_tokens(original), Ordering::Relaxed);
        self.tokens_out
            .fetch_add(estimate_tokens(compressed), Ordering::Relaxed);
    }

    /// Record a tool call that had no compressible text content.
    pub fn record_call(&self) {
        self.tool_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache hit (upstream not contacted), with the original and
    /// compressed token sizes of the cached response (used to accumulate
    /// compression savings across cache hits).
    pub fn record_cache_hit_with_tokens(&self, tokens_original: usize, tokens_compressed: usize) {
        self.tool_calls.fetch_add(1, Ordering::Relaxed);
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
        self.tokens_in.fetch_add(tokens_original, Ordering::Relaxed);
        self.tokens_out.fetch_add(tokens_compressed, Ordering::Relaxed);
    }

    /// Record a knowledge store hit (semantically similar past response found).
    pub fn record_knowledge_hit_with_tokens(
        &self,
        tokens_original: usize,
        tokens_compressed: usize,
    ) {
        self.tool_calls.fetch_add(1, Ordering::Relaxed);
        self.knowledge_hits.fetch_add(1, Ordering::Relaxed);
        self.tokens_in.fetch_add(tokens_original, Ordering::Relaxed);
        self.tokens_out
            .fetch_add(tokens_compressed, Ordering::Relaxed);
    }

    pub fn cache_hits(&self) -> usize {
        self.cache_hits.load(Ordering::Relaxed)
    }

    pub fn knowledge_hits(&self) -> usize {
        self.knowledge_hits.load(Ordering::Relaxed)
    }

    pub fn tool_calls(&self) -> usize {
        self.tool_calls.load(Ordering::Relaxed)
    }

    pub fn tokens_in(&self) -> usize {
        self.tokens_in.load(Ordering::Relaxed)
    }

    pub fn tokens_out(&self) -> usize {
        self.tokens_out.load(Ordering::Relaxed)
    }

    pub fn tokens_saved(&self) -> usize {
        self.tokens_in().saturating_sub(self.tokens_out())
    }

    pub fn savings_percent(&self) -> f64 {
        let input = self.tokens_in();
        if input == 0 {
            return 0.0;
        }
        (self.tokens_saved() as f64 / input as f64) * 100.0
    }

    /// Print session summary to stderr.
    pub fn print_summary(&self) {
        eprintln!();
        eprintln!("[trimcp] Session summary:");
        eprintln!("  Tool calls proxied : {}", self.tool_calls());
        eprintln!("  Cache hits         : {}", self.cache_hits());
        eprintln!("  Knowledge hits     : {}", self.knowledge_hits());
        eprintln!("  Tokens in          : {}", format_number(self.tokens_in()));
        eprintln!(
            "  Tokens out         : {}",
            format_number(self.tokens_out())
        );
        eprintln!(
            "  Saved              : {} ({:.1}%)",
            format_number(self.tokens_saved()),
            self.savings_percent()
        );
    }
}

fn format_number(n: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_accumulates_tokens() {
        let m = Metrics::new();
        m.record("hello world longer text here", "hello world");
        assert_eq!(m.tool_calls(), 1);
        assert!(m.tokens_in() > m.tokens_out());
    }

    #[test]
    fn test_tokens_saved_is_difference() {
        let m = Metrics::new();
        m.record("a".repeat(100).as_str(), "a".repeat(40).as_str());
        assert_eq!(m.tokens_saved(), m.tokens_in() - m.tokens_out());
    }

    #[test]
    fn test_savings_percent_zero_when_no_calls() {
        let m = Metrics::new();
        assert_eq!(m.savings_percent(), 0.0);
    }

    #[test]
    fn test_savings_percent_correct() {
        let m = Metrics::new();
        // 100 tokens in, 50 tokens out = 50%
        m.record("a".repeat(400).as_str(), "a".repeat(200).as_str());
        assert!((m.savings_percent() - 50.0).abs() < 1.0);
    }

    #[test]
    fn test_multiple_records_accumulate() {
        let m = Metrics::new();
        m.record("text one longer", "text one");
        m.record("text two longer", "text two");
        assert_eq!(m.tool_calls(), 2);
    }

    #[test]
    fn test_format_number_thousands() {
        assert_eq!(format_number(1_000), "1,000");
        assert_eq!(format_number(1_234_567), "1,234,567");
        assert_eq!(format_number(42), "42");
    }
}
