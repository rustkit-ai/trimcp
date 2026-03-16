#![allow(dead_code)]

/// Compress text output to reduce LLM token costs.
///
/// Goal: same information, fewer tokens. Never lose meaning, only reduce representation.
pub trait CompressionStrategy {
    fn compress(&self, text: &str) -> String;
}

/// Approximate token count (1 token ≈ 4 chars).
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Count tokens saved between original and compressed text.
pub fn tokens_saved(original: &str, compressed: &str) -> usize {
    estimate_tokens(original).saturating_sub(estimate_tokens(compressed))
}

// ── Strategies ────────────────────────────────────────────────────────────────

/// Remove ANSI escape codes (terminal colors, cursor control).
pub struct StripAnsi;

/// Minify pretty-printed JSON output.
pub struct CompactJson;

/// Remove single-line code comments (`//`, `#`).
pub struct StripComments;

/// Collapse consecutive duplicate lines into `line (xN)`.
pub struct Dedup;

/// Collapse multiple blank lines and trim trailing whitespace per line.
pub struct Minify;

// ── Implementations ───────────────────────────────────────────────────────────

impl CompressionStrategy for StripAnsi {
    fn compress(&self, text: &str) -> String {
        let mut result = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                // Skip ESC sequence: ESC [ ... final_byte (0x40–0x7E)
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            break;
                        }
                    }
                }
            } else {
                result.push(ch);
            }
        }

        result
    }
}

impl CompressionStrategy for CompactJson {
    fn compress(&self, text: &str) -> String {
        let trimmed = text.trim();
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(value) => serde_json::to_string(&value).unwrap_or_else(|_| text.to_string()),
            Err(_) => text.to_string(),
        }
    }
}

impl CompressionStrategy for StripComments {
    fn compress(&self, text: &str) -> String {
        let mut in_code_block = false;
        text.lines()
            .filter(|line| {
                let trimmed = line.trim();
                if trimmed.starts_with("```") {
                    in_code_block = !in_code_block;
                }
                // Strip `//` full-line comments only inside code blocks.
                // Never strip `#` — it's a Markdown header outside code blocks
                // and a meaningful comment (shell/Python) inside.
                !(in_code_block && trimmed.starts_with("//"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl CompressionStrategy for Dedup {
    fn compress(&self, text: &str) -> String {
        let mut result: Vec<String> = Vec::new();
        let mut current: Option<&str> = None;
        let mut count = 0usize;

        for line in text.lines() {
            match current {
                Some(prev) if prev == line => {
                    count += 1;
                }
                _ => {
                    if let Some(prev) = current {
                        if count > 1 {
                            result.push(format!("{prev} (x{count})"));
                        } else {
                            result.push(prev.to_string());
                        }
                    }
                    current = Some(line);
                    count = 1;
                }
            }
        }

        if let Some(prev) = current {
            if count > 1 {
                result.push(format!("{prev} (x{count})"));
            } else {
                result.push(prev.to_string());
            }
        }

        result.join("\n")
    }
}

impl CompressionStrategy for Minify {
    fn compress(&self, text: &str) -> String {
        let mut result: Vec<&str> = Vec::new();
        let mut blank_count = 0usize;

        for line in text.lines() {
            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                blank_count += 1;
                if blank_count == 1 {
                    result.push("");
                }
            } else {
                blank_count = 0;
                result.push(trimmed);
            }
        }

        result.join("\n")
    }
}

/// Apply a pipeline of strategies in sequence.
pub struct Pipeline {
    strategies: Vec<Box<dyn CompressionStrategy>>,
}

impl Pipeline {
    pub fn default_pipeline() -> Self {
        Self {
            strategies: vec![
                Box::new(StripAnsi),
                Box::new(CompactJson),
                Box::new(StripComments),
                Box::new(Dedup),
                Box::new(Minify),
            ],
        }
    }

    pub fn compress(&self, text: &str) -> String {
        self.strategies
            .iter()
            .fold(text.to_string(), |acc, s| s.compress(&acc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── StripAnsi ─────────────────────────────────────────────────────────────

    #[test]
    fn test_strip_ansi_removes_color_codes() {
        let input = "\x1b[32mhello\x1b[0m world";
        assert_eq!(StripAnsi.compress(input), "hello world");
    }

    #[test]
    fn test_strip_ansi_plain_text_unchanged() {
        let input = "plain text without ansi";
        assert_eq!(StripAnsi.compress(input), input);
    }

    // ── CompactJson ───────────────────────────────────────────────────────────

    #[test]
    fn test_compact_json_minifies_pretty_json() {
        let input = "{\n  \"key\": \"value\",\n  \"num\": 42\n}";
        let output = CompactJson.compress(input);
        assert_eq!(output, r#"{"key":"value","num":42}"#);
        assert!(output.len() < input.len());
    }

    #[test]
    fn test_compact_json_leaves_non_json_unchanged() {
        let input = "this is not json";
        assert_eq!(CompactJson.compress(input), input);
    }

    // ── StripComments ─────────────────────────────────────────────────────────

    #[test]
    fn test_strip_comments_removes_slash_comments_in_code_block() {
        let input = "```rust\nlet x = 1;\n// this is a comment\nlet y = 2;\n```";
        let output = StripComments.compress(input);
        assert!(!output.contains("// this is a comment"));
        assert!(output.contains("let x = 1;"));
        assert!(output.contains("let y = 2;"));
    }

    #[test]
    fn test_strip_comments_keeps_hash_markdown_headers() {
        let input = "# Title\n## Section\nsome text";
        let output = StripComments.compress(input);
        assert!(output.contains("# Title"));
        assert!(output.contains("## Section"));
    }

    #[test]
    fn test_strip_comments_keeps_slash_comments_outside_code_block() {
        // `//` outside a code block is kept (could be part of prose or URLs)
        let input = "// not in a code block";
        assert_eq!(StripComments.compress(input), input);
    }

    #[test]
    fn test_strip_comments_keeps_inline_comments() {
        // Only strips full-line comments, not inline ones
        let input = "```rust\nlet x = 1; // inline comment\n```";
        assert!(StripComments.compress(input).contains("// inline comment"));
    }

    // ── Dedup ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_dedup_collapses_consecutive_duplicates() {
        let input = "INFO started\nINFO started\nINFO started\nERROR failed";
        let output = Dedup.compress(input);
        assert_eq!(output, "INFO started (x3)\nERROR failed");
    }

    #[test]
    fn test_dedup_keeps_non_consecutive_duplicates() {
        let input = "line a\nline b\nline a";
        let output = Dedup.compress(input);
        assert_eq!(output, "line a\nline b\nline a");
    }

    #[test]
    fn test_dedup_single_line_unchanged() {
        let input = "only one line";
        assert_eq!(Dedup.compress(input), input);
    }

    // ── Minify ────────────────────────────────────────────────────────────────

    #[test]
    fn test_minify_collapses_multiple_blank_lines() {
        let input = "line one\n\n\n\nline two";
        let output = Minify.compress(input);
        assert_eq!(output, "line one\n\nline two");
    }

    #[test]
    fn test_minify_trims_trailing_whitespace() {
        let input = "line one   \nline two  ";
        let output = Minify.compress(input);
        assert_eq!(output, "line one\nline two");
    }

    // ── estimate_tokens ───────────────────────────────────────────────────────

    #[test]
    fn test_estimate_tokens_approximation() {
        let text = "hello world"; // 11 chars → 3 tokens
        assert_eq!(estimate_tokens(text), 3);
    }

    #[test]
    fn test_tokens_saved_returns_difference() {
        let original = "a".repeat(100);
        let compressed = "a".repeat(40);
        assert_eq!(tokens_saved(&original, &compressed), 15);
    }

    // ── Pipeline ──────────────────────────────────────────────────────────────

    #[test]
    fn test_pipeline_applies_all_strategies() {
        let input = "\x1b[32mINFO started\x1b[0m\nINFO started\nINFO started\n\n\n";
        let output = Pipeline::default_pipeline().compress(input);
        assert!(!output.contains("\x1b["));
        assert!(output.contains("(x3)"));
        assert!(!output.ends_with("\n\n"));
    }
}
