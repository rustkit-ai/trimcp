#![cfg(feature = "semtree")]

use std::path::Path;
use std::sync::Arc;

use semtree_core::Chunk;
use semtree_embed::fastembed::FastEmbedder;
use semtree_rag::{ChunkRegistry, Indexer, SearchEngine};
use semtree_store::usearch::UsearchStore;
use semtree_store::VectorStore;
use tracing::{info, warn};

use crate::error::{Error, Result};

/// Wraps a semtree index (vector store + chunk registry) and exposes semantic
/// code search for use in the proxy enrichment pipeline.
///
/// # Lifecycle
///
/// 1. **Index once** with [`CodeContext::build`] — parses a codebase directory,
///    embeds every code chunk, and persists the index to disk.
/// 2. **Load on proxy start** with [`CodeContext::load`] — restores the index
///    from disk without re-embedding.
/// 3. **Search at runtime** with [`CodeContext::search`] — returns the `top_k`
///    code chunks most relevant to a natural-language query.
pub struct CodeContext {
    engine: Arc<SearchEngine>,
    registry: ChunkRegistry,
    top_k: usize,
}

impl CodeContext {
    /// Index `codebase_dir` and persist the result to `index_dir`.
    ///
    /// Downloads the embedding model on first call (~23 MB, cached locally by
    /// fastembed). Subsequent calls reuse the cached model.
    pub async fn build(codebase_dir: &Path, index_dir: &Path, top_k: usize) -> Result<Self> {
        std::fs::create_dir_all(index_dir)
            .map_err(|e| Error::Config(format!("cannot create semtree index dir: {e}")))?;

        let embedder = Arc::new(
            FastEmbedder::new()
                .map_err(|e| Error::Upstream(format!("semtree embedder init: {e}")))?,
        );
        let store = Arc::new(
            UsearchStore::new(384)
                .map_err(|e| Error::Upstream(format!("semtree store init: {e}")))?,
        );

        let indexer = Indexer::new(embedder.clone(), store.clone());
        let mut registry = ChunkRegistry::default();

        let n = indexer
            .index_dir(codebase_dir, &mut registry)
            .await
            .map_err(|e| Error::Upstream(format!("semtree index_dir: {e}")))?;

        info!(chunks = n, path = %codebase_dir.display(), "semtree index built");

        store
            .save(index_dir)
            .map_err(|e| Error::Upstream(format!("semtree store save: {e}")))?;
        registry
            .save(index_dir)
            .map_err(|e| Error::Upstream(format!("semtree registry save: {e}")))?;

        let engine = Arc::new(SearchEngine::new(embedder, store));
        Ok(Self { engine, registry, top_k })
    }

    /// Load a pre-built index from `index_dir`.
    ///
    /// Returns an error if the index has not yet been built — run
    /// `trimcp semtree-index <name> <path>` first.
    pub fn load(index_dir: &Path, top_k: usize) -> Result<Self> {
        if !index_dir.exists() {
            return Err(Error::Config(format!(
                "semtree index not found at {} — run `trimcp semtree-index <name> <path>` first",
                index_dir.display()
            )));
        }

        let embedder = Arc::new(
            FastEmbedder::new()
                .map_err(|e| Error::Upstream(format!("semtree embedder init: {e}")))?,
        );

        let mut store = UsearchStore::new(384)
            .map_err(|e| Error::Upstream(format!("semtree store init: {e}")))?;
        store
            .load(index_dir)
            .map_err(|e| Error::Upstream(format!("semtree store load: {e}")))?;
        let store = Arc::new(store);

        let mut registry = ChunkRegistry::default();
        registry
            .load(index_dir)
            .map_err(|e| Error::Upstream(format!("semtree registry load: {e}")))?;

        let engine = Arc::new(SearchEngine::new(embedder, store));
        Ok(Self { engine, registry, top_k })
    }

    /// Search for code chunks semantically relevant to `query`.
    ///
    /// Returns up to `top_k` chunks, silently returning an empty vec on error.
    pub async fn search(&self, query: &str) -> Vec<Chunk> {
        match self.engine.search(query, self.top_k).await {
            Ok(hits) => hits
                .iter()
                .filter_map(|h| self.registry.get(&h.id))
                .cloned()
                .collect(),
            Err(e) => {
                warn!(err = %e, "semtree search failed");
                vec![]
            }
        }
    }

    /// Number of indexed chunks.
    pub fn len(&self) -> usize {
        self.registry.len()
    }

    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semtree_core::{ChunkKind, Language, Span};
    use std::path::PathBuf;

    fn make_chunk(name: &str, content: &str) -> Chunk {
        Chunk {
            id: name.to_string(),
            path: PathBuf::from(format!("src/{name}.rs")),
            language: Language::Rust,
            kind: ChunkKind::Function,
            name: Some(name.to_string()),
            content: content.to_string(),
            span: Span::new(0, content.len(), 10, 15),
            doc: Some(format!("/// {name} docs")),
        }
    }

    #[test]
    fn test_format_code_context_empty() {
        let result = format_code_context(&[]);
        assert_eq!(result, "[Code Context]");
    }

    #[test]
    fn test_format_code_context_single_chunk() {
        let chunk = make_chunk("authenticate", "pub fn authenticate() {}");
        let result = format_code_context(&[chunk]);
        assert!(result.contains("[Code Context]"));
        assert!(result.contains("authenticate"));
        assert!(result.contains("src/authenticate.rs:10–15"));
        assert!(result.contains("rust"));
        assert!(result.contains("pub fn authenticate() {}"));
    }

    #[test]
    fn test_format_code_context_truncates_at_six_lines() {
        let content = (0..20)
            .map(|i| format!("  let x{i} = {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        let chunk = make_chunk("big_fn", &content);
        let result = format_code_context(&[chunk]);
        // Only first 6 lines of content should appear
        let content_lines: Vec<&str> = result.lines().skip(2).collect(); // skip header + comment
        assert!(content_lines.len() <= 7); // 6 content lines + possible blank
    }

    #[test]
    fn test_format_code_context_multiple_chunks() {
        let chunks = vec![
            make_chunk("fn_a", "pub fn fn_a() {}"),
            make_chunk("fn_b", "pub fn fn_b() {}"),
        ];
        let result = format_code_context(&chunks);
        assert!(result.contains("fn_a"));
        assert!(result.contains("fn_b"));
    }

    #[test]
    fn test_format_code_context_unnamed_chunk() {
        let mut chunk = make_chunk("x", "fn x() {}");
        chunk.name = None;
        let result = format_code_context(&[chunk]);
        assert!(result.contains("(unnamed)"));
    }

    #[test]
    fn test_load_returns_error_when_index_missing() {
        let nonexistent = PathBuf::from("/tmp/semtree_test_nonexistent_xyz_abc");
        let result = CodeContext::load(&nonexistent, 3);
        let err = result.err().expect("expected an error for missing index");
        assert!(err.to_string().contains("semtree index not found"));
    }

    /// Full index + search round-trip. Skipped in CI — downloads fastembed model (~23 MB).
    /// Run manually with: `cargo test --features semtree -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn test_build_and_search_roundtrip() {
        use std::fs;
        let tmp = std::env::temp_dir().join("semtree_trimcp_test");
        let codebase = std::env::temp_dir().join("semtree_trimcp_src");
        fs::create_dir_all(&codebase).unwrap();
        fs::write(
            codebase.join("lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        )
        .unwrap();

        let ctx = CodeContext::build(&codebase, &tmp, 3)
            .await
            .expect("build failed");
        assert!(!ctx.is_empty());

        let hits = ctx.search("arithmetic addition").await;
        assert!(!hits.is_empty());

        let _ = fs::remove_dir_all(&tmp);
        let _ = fs::remove_dir_all(&codebase);
    }
}

/// Format a slice of code chunks as a compact `[Code Context]` block suitable
/// for prepending to an MCP tool response.
pub fn format_code_context(chunks: &[Chunk]) -> String {
    let mut lines = vec!["[Code Context]".to_string()];
    for chunk in chunks {
        let name = chunk.name.as_deref().unwrap_or("(unnamed)");
        let path = chunk.path.display();
        let lang = format!("{:?}", chunk.language).to_lowercase();
        let span = format!("{}–{}", chunk.span.start_line, chunk.span.end_line);
        lines.push(format!("// {name} — {path}:{span} ({lang})"));
        let preview: String = chunk
            .content
            .lines()
            .take(6)
            .collect::<Vec<_>>()
            .join("\n");
        lines.push(preview);
        lines.push(String::new());
    }
    lines.join("\n")
}
