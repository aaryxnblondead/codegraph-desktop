//! Storage abstraction over code-graph data.
//!
//! `Storage` is a trait with an associated `Error` type. Implementations
//! return owned data, so callers never borrow from a connection.
//!
//! Implementations:
//! - `MemoryStorage`: wraps an in-memory [`CodeGraph`].
//! - `SqliteStorage` (feature `sqlite`): reads a legacy `codegraph.db`.
//! - `IndexStorage` (in `storage`) will read the app's own cache.

use std::path::PathBuf;

use std::collections::HashMap;

use crate::error::CodegraphError;
use crate::graph::CodeGraph;
use crate::model::{FileInfo, FileStatus, Relationship, RelationshipKind, Symbol, SymbolKind};

use crate::model::SymbolId;

/// Abstraction over a source of code-graph data.
pub trait Storage {
    type Error: std::error::Error + Send + Sync;

    /// Every symbol (node) in the graph.
    fn symbols(&self) -> Result<Vec<Symbol>, Self::Error>;

    /// Every relationship (edge) in the graph.
    fn relationships(&self) -> Result<Vec<Relationship>, Self::Error>;

    /// Per-file information. Defaults to empty for sources that don't track it.
    fn files(&self) -> Result<HashMap<String, FileInfo>, Self::Error> {
        Ok(HashMap::new())
    }
}

/// In-memory storage backed by an already-loaded [`CodeGraph`].
pub struct MemoryStorage {
    graph: CodeGraph,
}

impl MemoryStorage {
    pub fn new(graph: CodeGraph) -> Self {
        Self { graph }
    }
}

impl Storage for MemoryStorage {
    type Error = CodegraphError;

    fn symbols(&self) -> Result<Vec<Symbol>, Self::Error> {
        Ok(self.graph.symbols().to_vec())
    }

    fn relationships(&self) -> Result<Vec<Relationship>, Self::Error> {
        Ok(self.graph.relationships().to_vec())
    }

    fn files(&self) -> Result<HashMap<String, FileInfo>, Self::Error> {
        Ok(self.graph.files().clone())
    }
}

/// Derive a per-file map from a symbol list (legacy DBs have no files table).
pub fn derive_files(symbols: &[Symbol]) -> HashMap<String, FileInfo> {
    let mut files: HashMap<String, FileInfo> = HashMap::new();
    for s in symbols {
        let entry = files.entry(s.file.clone()).or_insert_with(|| FileInfo {
            path: s.file.clone(),
            lang: s.lang.clone(),
            loc: 0,
            symbol_count: 0,
            status: FileStatus::Indexed,
            complexity_sum: 0,
            complexity_max: 0,
            function_count: 0,
            ignores: Vec::new(),
        });
        entry.symbol_count += 1;
        if entry.lang.is_none() {
            entry.lang = s.lang.clone();
        }
    }
    files
}

/// Storage backed by the on-disk index cache (loads the graph at open).
pub struct IndexStorage {
    graph: CodeGraph,
}

impl IndexStorage {
    pub fn open(cache_path: impl Into<PathBuf>) -> Result<Self, CodegraphError> {
        let cache = crate::indexer::cache::IndexCache::open(cache_path)?;
        let graph = cache.load_graph()?.unwrap_or_default();
        Ok(Self { graph })
    }

    pub fn open_project(root: &std::path::Path) -> Result<Self, CodegraphError> {
        Self::open(crate::indexer::cache::cache_path_for(root))
    }

    pub fn graph(&self) -> &CodeGraph {
        &self.graph
    }
}

impl Storage for IndexStorage {
    type Error = CodegraphError;

    fn symbols(&self) -> Result<Vec<Symbol>, Self::Error> {
        Ok(self.graph.symbols().to_vec())
    }

    fn relationships(&self) -> Result<Vec<Relationship>, Self::Error> {
        Ok(self.graph.relationships().to_vec())
    }

    fn files(&self) -> Result<HashMap<String, FileInfo>, Self::Error> {
        Ok(self.graph.files().clone())
    }
}

/// Reads a CodeGraph knowledge graph from a legacy SQLite database.
///
/// Tables: `nodes(id, name, kind, file_path, start_line, signature, docstring)`
/// and `edges(id, source, target, kind)`.
pub struct SqliteStorage {
    conn: rusqlite::Connection,
}

impl SqliteStorage {
    /// Open the database at `path`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, CodegraphError> {
        let path = path.into();
        let conn = rusqlite::Connection::open(&path).map_err(CodegraphError::Database)?;
        Ok(Self { conn })
    }
}

impl Storage for SqliteStorage {
    type Error = CodegraphError;

    fn symbols(&self) -> Result<Vec<Symbol>, Self::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, file_path, start_line, signature, docstring FROM nodes",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let kind: String = row.get(2)?;
            let file: String = row.get(3)?;
            let mut sym = Symbol::new(SymbolId(id), name, SymbolKind::parse(&kind), file);
            sym.line = row.get::<_, Option<i64>>(4)?.map(|v| v as u32);
            sym.signature = row.get(5)?;
            sym.doc = row.get(6)?;
            Ok(sym)
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(CodegraphError::Database)?);
        }
        Ok(out)
    }

    fn relationships(&self) -> Result<Vec<Relationship>, Self::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, source, target, kind FROM edges")?;
        let rows = stmt.query_map([], |row| {
            let kind: String = row.get(3)?;
            let mut rel = Relationship::new(
                row.get::<_, i64>(0)? as u64,
                SymbolId(row.get::<_, String>(1)?),
                SymbolId(row.get::<_, String>(2)?),
                RelationshipKind::parse(&kind),
            );
            rel.provenance = "legacy".to_string();
            Ok(rel)
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(CodegraphError::Database)?);
        }
        Ok(out)
    }

    fn files(&self) -> Result<HashMap<String, FileInfo>, Self::Error> {
        Ok(derive_files(&self.symbols()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeDirection, Relationship, Symbol, SymbolId};

    fn sym(id: &str, name: &str) -> Symbol {
        Symbol::new(SymbolId(id.into()), name, Default::default(), "src/lib.rs")
    }

    #[test]
    fn memory_storage_round_trip() {
        let graph = CodeGraph::new(
            vec![sym("1", "alpha"), sym("2", "beta")],
            vec![Relationship::new(
                1,
                SymbolId("1".into()),
                SymbolId("2".into()),
                RelationshipKind::Calls,
            )],
        )
        .expect("valid graph");
        let store = MemoryStorage::new(graph);

        assert_eq!(store.symbols().unwrap().len(), 2);
        assert_eq!(store.relationships().unwrap().len(), 1);
        assert!(store.symbols().unwrap().iter().any(|s| s.name == "alpha"));
    }

    #[test]
    fn derive_files_counts_symbols() {
        let files = derive_files(&[sym("1", "a"), sym("2", "b")]);
        assert_eq!(files.get("src/lib.rs").map(|f| f.symbol_count), Some(2));
    }

    #[test]
    fn relationships_for_directions() {
        let graph = CodeGraph::new(
            vec![sym("1", "a"), sym("2", "b")],
            vec![Relationship::new(
                1,
                SymbolId("1".into()),
                SymbolId("2".into()),
                RelationshipKind::Calls,
            )],
        )
        .expect("valid graph");
        assert_eq!(
            graph
                .relationships_for(SymbolId("1".into()), EdgeDirection::Outgoing)
                .len(),
            1
        );
    }
}
