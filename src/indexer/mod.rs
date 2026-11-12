//! Self-indexing pipeline: walk → extract → resolve → `CodeGraph`.
//!
//! Heavy work runs off the UI thread; progress and the final graph are sent
//! back through a channel. Per-file extraction is wrapped in `catch_unwind` so
//! one bad file never aborts the run.

pub mod cache;
pub mod facts;
pub mod lang;
pub mod resolve;
pub mod walk;

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use crate::config::ProjectConfig;
use crate::error::CodegraphError;
use crate::graph::CodeGraph;
use crate::paths::normalize;

use facts::FileFacts;

#[derive(Debug, Clone)]
pub enum IndexProgress {
    Walking { found: usize },
    Parsing { done: usize, total: usize },
    Resolving,
    Writing,
    Done(IndexSummary),
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Default)]
pub struct IndexSummary {
    pub files: usize,
    pub parsed: usize,
    pub cached: usize,
    pub skipped: usize,
    pub failed: usize,
    pub symbols: usize,
    pub edges: usize,
    pub refs_total: usize,
    pub refs_resolved: usize,
    pub duration_ms: u128,
    pub notes: Vec<String>,
}

impl IndexSummary {
    pub fn resolved_ratio(&self) -> f32 {
        if self.refs_total == 0 {
            0.0
        } else {
            self.refs_resolved as f32 / self.refs_total as f32
        }
    }
}

pub struct IndexOutcome {
    pub graph: CodeGraph,
    pub summary: IndexSummary,
}

/// Index `root` into a `CodeGraph` (no persistent cache).
pub fn index_project(
    root: &Path,
    config: &ProjectConfig,
    progress: Option<Sender<IndexProgress>>,
    cancel: Arc<AtomicBool>,
) -> Result<IndexOutcome, CodegraphError> {
    index_project_cached(root, config, progress, cancel, None)
}

/// Index `root`, reusing cached facts when `cache_path` is provided.
pub fn index_project_cached(
    root: &Path,
    config: &ProjectConfig,
    progress: Option<Sender<IndexProgress>>,
    cancel: Arc<AtomicBool>,
    cache_path: Option<&Path>,
) -> Result<IndexOutcome, CodegraphError> {
    let started = Instant::now();
    let mut summary = IndexSummary::default();
    let mut cache = match cache_path {
        Some(p) => Some(cache::IndexCache::open(p)?),
        None => None,
    };

    let discovered = walk::walk_files(root, &config.ignore);
    summary.skipped = discovered.skipped_large.len() + discovered.skipped_ignored.len();
    if let Some(tx) = &progress {
        let _ = tx.send(IndexProgress::Walking {
            found: discovered.files.len(),
        });
    }

    let supported: Vec<_> = discovered
        .files
        .iter()
        .filter(|p| lang::extractor_for(p).is_some())
        .cloned()
        .collect();
    let total = supported.len();
    summary.files = total;

    let mut facts_map: HashMap<String, FileFacts> = HashMap::with_capacity(total);
    let mut seen: HashSet<String> = HashSet::with_capacity(total);
    let mut done = 0usize;

    for path in &supported {
        if cancel.load(Ordering::Relaxed) {
            if let Some(tx) = &progress {
                let _ = tx.send(IndexProgress::Cancelled);
            }
            return Err(CodegraphError::Index("cancelled".into()));
        }
        let Some(extractor) = lang::extractor_for(path) else {
            continue;
        };
        let rel = normalize(root, path);
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                summary.failed += 1;
                summary.notes.push(format!("failed to read {rel}: {e}"));
                done += 1;
                continue;
            }
        };
        seen.insert(rel.clone());
        let hash = blake3::hash(src.as_bytes()).to_hex().to_string();

        // Reuse cached facts when the content hash is unchanged.
        if let Some(c) = &cache {
            if let Ok(Some(facts)) = c.cached_facts(&rel, &hash) {
                facts_map.insert(rel, facts);
                summary.cached += 1;
                done += 1;
                continue;
            }
        }

        let result = catch_unwind(AssertUnwindSafe(|| extractor.extract(path, &src)));
        match result {
            Ok(facts) => {
                if let Some(c) = &cache {
                    let loc = facts.metrics.loc;
                    let (size, mtime) = std::fs::metadata(path)
                        .map(|m| {
                            let mtime = m
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0);
                            (m.len(), mtime)
                        })
                        .unwrap_or((src.len() as u64, 0));
                    let _ = c.upsert_file(
                        &rel,
                        &hash,
                        cache::FileMeta {
                            lang: Some(extractor.language_id()),
                            loc,
                            size,
                            mtime,
                        },
                        &facts,
                    );
                }
                facts_map.insert(rel, facts);
                summary.parsed += 1;
            }
            Err(_) => {
                summary.failed += 1;
                summary.notes.push(format!("extractor panicked on {rel}"));
            }
        }
        done += 1;
        if done.is_multiple_of(50) {
            if let Some(tx) = &progress {
                let _ = tx.send(IndexProgress::Parsing { done, total });
            }
        }
    }

    if let Some(c) = &mut cache {
        let _ = c.prune(&seen);
    }

    if cancel.load(Ordering::Relaxed) {
        if let Some(tx) = &progress {
            let _ = tx.send(IndexProgress::Cancelled);
        }
        return Err(CodegraphError::Index("cancelled".into()));
    }

    if let Some(tx) = &progress {
        let _ = tx.send(IndexProgress::Resolving);
    }
    let resolved = resolve::resolve(root, &facts_map, config);
    summary.symbols = resolved.symbols.len();
    summary.edges = resolved.relationships.len();
    summary.refs_total = resolved.refs_total;
    summary.refs_resolved = resolved.refs_resolved;
    if let Some(tx) = &progress {
        let _ = tx.send(IndexProgress::Writing);
    }

    if let Some(c) = &mut cache {
        let _ = c.replace_graph(&resolved.symbols, &resolved.relationships, &resolved.files);
    }

    let graph =
        CodeGraph::new_with_files(resolved.symbols, resolved.relationships, resolved.files)?;

    summary.duration_ms = started.elapsed().as_millis();
    if summary.failed > 0 {
        summary
            .notes
            .push(format!("{} file(s) failed to parse", summary.failed));
    }
    let ratio = summary.resolved_ratio();
    if summary.refs_total > 0 && ratio < 0.5 {
        summary.notes.push(format!(
            "only {:.0}% of references resolved ({} of {}); method calls on \
             external types are not guessed, so low-confidence edges are filtered",
            ratio * 100.0,
            summary.refs_resolved,
            summary.refs_total
        ));
    }
    if let Some(tx) = &progress {
        let _ = tx.send(IndexProgress::Done(summary.clone()));
    }
    Ok(IndexOutcome { graph, summary })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn touch(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn indexes_a_small_ts_project() {
        let dir = std::env::temp_dir().join("cg_index_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        touch(&dir, "src/a.ts", "export function foo() { return 1; }\n");
        touch(
            &dir,
            "src/b.ts",
            "import { foo } from './a';\nexport function bar() { return foo(); }\n",
        );

        let outcome = index_project(
            &dir,
            &ProjectConfig::default(),
            None,
            Arc::new(AtomicBool::new(false)),
        )
        .expect("index");

        assert!(outcome.graph.symbol_count() >= 4, "symbols");
        assert!(outcome.graph.relationship_count() >= 2, "edges");
        assert_eq!(outcome.summary.files, 2);
        assert!(outcome
            .graph
            .relationships()
            .iter()
            .any(|r| r.kind == crate::model::RelationshipKind::Imports));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancellation_stops_early() {
        let dir = std::env::temp_dir().join("cg_index_cancel");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        touch(&dir, "a.ts", "export function a() {}\n");
        let cancel = Arc::new(AtomicBool::new(true));
        let result = index_project(&dir, &ProjectConfig::default(), None, cancel);
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incremental_reuses_cached_facts_and_persists_graph() {
        let dir = std::env::temp_dir().join("cg_index_incremental");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        touch(&dir, "src/a.ts", "export function foo() { return 1; }\n");
        touch(&dir, "src/b.ts", "export function bar() { return 2; }\n");
        let cache_path = dir.join("index.sqlite");
        let cfg = ProjectConfig::default();

        let first = index_project_cached(
            &dir,
            &cfg,
            None,
            Arc::new(AtomicBool::new(false)),
            Some(&cache_path),
        )
        .expect("first index");
        assert_eq!(first.summary.parsed, 2);
        assert_eq!(first.summary.cached, 0);

        // Change one file, re-index: one parsed, one reused.
        touch(&dir, "src/b.ts", "export function bar() { return 3; }\n");
        let second = index_project_cached(
            &dir,
            &cfg,
            None,
            Arc::new(AtomicBool::new(false)),
            Some(&cache_path),
        )
        .expect("second index");
        assert_eq!(second.summary.parsed, 1, "one file changed");
        assert_eq!(second.summary.cached, 1, "one file reused");

        // The persisted graph matches the in-memory result.
        let cache = crate::indexer::cache::IndexCache::open(&cache_path).unwrap();
        let loaded = cache.load_graph().unwrap().expect("cached graph");
        assert_eq!(loaded.symbol_count(), second.graph.symbol_count());
        assert_eq!(
            loaded.relationship_count(),
            second.graph.relationship_count()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unchanged_second_index_reuses_everything() {
        let dir = std::env::temp_dir().join("cg_index_reuse_all");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        touch(&dir, "src/a.ts", "export function foo() {}\n");
        let cache_path = dir.join("index.sqlite");
        let cfg = ProjectConfig::default();
        let _ = index_project_cached(
            &dir,
            &cfg,
            None,
            Arc::new(AtomicBool::new(false)),
            Some(&cache_path),
        )
        .unwrap();
        let second = index_project_cached(
            &dir,
            &cfg,
            None,
            Arc::new(AtomicBool::new(false)),
            Some(&cache_path),
        )
        .unwrap();
        assert_eq!(second.summary.parsed, 0);
        assert_eq!(second.summary.cached, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
