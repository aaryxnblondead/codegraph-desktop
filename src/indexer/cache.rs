//! On-disk index cache (SQLite).
//!
//! Stores per-file facts keyed by content hash so unchanged files are not
//! re-parsed, and stores the resolved graph so startup can show the last
//! project immediately. A schema/extractor version mismatch clears the cache.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::error::CodegraphError;
use crate::graph::CodeGraph;
use crate::model::{
    FileInfo, FileStatus, Relationship, RelationshipKind, Symbol, SymbolId, SymbolKind,
};

use super::facts::FileFacts;

pub const SCHEMA_VERSION: i64 = 3;
/// Bump when extractor output changes so stale facts are discarded.
pub const EXTRACTOR_VERSION: &str = "2";

pub struct IndexCache {
    conn: Connection,
    path: PathBuf,
}

/// Metadata stored alongside cached facts.
#[derive(Debug, Clone, Copy)]
pub struct FileMeta<'a> {
    pub lang: Option<&'a str>,
    pub loc: usize,
    pub size: u64,
    pub mtime: i64,
}

impl IndexCache {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, CodegraphError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path).map_err(CodegraphError::Database)?;
        let mut cache = Self { conn, path };
        cache.init()?;
        Ok(cache)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn init(&mut self) -> Result<(), CodegraphError> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS files(
                   path TEXT PRIMARY KEY, hash TEXT NOT NULL, lang TEXT,
                   loc INTEGER, size INTEGER, mtime INTEGER, facts TEXT NOT NULL);
                 CREATE TABLE IF NOT EXISTS nodes(
                   id TEXT PRIMARY KEY, name TEXT, kind TEXT, file_path TEXT,
                   start_line INTEGER, end_line INTEGER, signature TEXT, docstring TEXT,
                   qualified_name TEXT, exported INTEGER, lang TEXT,
                   complexity INTEGER, render INTEGER);
                 CREATE TABLE IF NOT EXISTS edges(
                   id INTEGER PRIMARY KEY, source TEXT, target TEXT, kind TEXT,
                   line INTEGER, confidence REAL, provenance TEXT);
                 CREATE TABLE IF NOT EXISTS file_info(
                   path TEXT PRIMARY KEY, lang TEXT, loc INTEGER, symbol_count INTEGER,
                   status TEXT, complexity_sum INTEGER, complexity_max INTEGER,
                   function_count INTEGER, ignores TEXT);",
            )
            .map_err(CodegraphError::Database)?;

        let schema = self.get_meta("schema_version")?;
        let extractor = self.get_meta("extractor_version")?;
        if schema.as_deref() != Some(&SCHEMA_VERSION.to_string())
            || extractor.as_deref() != Some(EXTRACTOR_VERSION)
        {
            self.clear()?;
            self.set_meta("schema_version", &SCHEMA_VERSION.to_string())?;
            self.set_meta("extractor_version", EXTRACTOR_VERSION)?;
        }
        Ok(())
    }

    pub fn clear(&mut self) -> Result<(), CodegraphError> {
        self.conn
            .execute_batch(
                "DELETE FROM files; DELETE FROM nodes; DELETE FROM edges; DELETE FROM file_info;",
            )
            .map_err(CodegraphError::Database)
    }

    fn get_meta(&self, key: &str) -> Result<Option<String>, CodegraphError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM meta WHERE key = ?1")
            .map_err(CodegraphError::Database)?;
        let mut rows = stmt.query(params![key]).map_err(CodegraphError::Database)?;
        match rows.next().map_err(CodegraphError::Database)? {
            Some(row) => Ok(Some(
                row.get::<_, String>(0).map_err(CodegraphError::Database)?,
            )),
            None => Ok(None),
        }
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<(), CodegraphError> {
        self.conn
            .execute(
                "INSERT INTO meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(CodegraphError::Database)?;
        Ok(())
    }

    /// Cached facts for `path` if the stored hash matches `hash`.
    pub fn cached_facts(
        &self,
        path: &str,
        hash: &str,
    ) -> Result<Option<FileFacts>, CodegraphError> {
        let mut stmt = self
            .conn
            .prepare("SELECT facts FROM files WHERE path = ?1 AND hash = ?2")
            .map_err(CodegraphError::Database)?;
        let mut rows = stmt
            .query(params![path, hash])
            .map_err(CodegraphError::Database)?;
        if let Some(row) = rows.next().map_err(CodegraphError::Database)? {
            let text: String = row.get(0).map_err(CodegraphError::Database)?;
            if let Ok(facts) = serde_json::from_str::<FileFacts>(&text) {
                return Ok(Some(facts));
            }
        }
        Ok(None)
    }

    pub fn upsert_file(
        &self,
        path: &str,
        hash: &str,
        meta: FileMeta<'_>,
        facts: &FileFacts,
    ) -> Result<(), CodegraphError> {
        let text =
            serde_json::to_string(facts).map_err(|e| CodegraphError::Index(e.to_string()))?;
        self.conn
            .execute(
                "INSERT INTO files(path, hash, lang, loc, size, mtime, facts)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(path) DO UPDATE SET
                   hash=excluded.hash, lang=excluded.lang, loc=excluded.loc,
                   size=excluded.size, mtime=excluded.mtime, facts=excluded.facts",
                params![
                    path,
                    hash,
                    meta.lang,
                    meta.loc as i64,
                    meta.size as i64,
                    meta.mtime,
                    text
                ],
            )
            .map_err(CodegraphError::Database)?;
        Ok(())
    }

    /// Remove cached files that no longer exist in the project.
    pub fn prune(&mut self, existing: &HashSet<String>) -> Result<(), CodegraphError> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM files")
            .map_err(CodegraphError::Database)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(CodegraphError::Database)?;
        let mut stale = Vec::new();
        for row in rows {
            let path = row.map_err(CodegraphError::Database)?;
            if !existing.contains(&path) {
                stale.push(path);
            }
        }
        for path in stale {
            self.conn
                .execute("DELETE FROM files WHERE path = ?1", params![path])
                .map_err(CodegraphError::Database)?;
        }
        Ok(())
    }

    /// Persist the resolved graph (transactional replace).
    pub fn replace_graph(
        &mut self,
        symbols: &[Symbol],
        relationships: &[Relationship],
        files: &std::collections::HashMap<String, FileInfo>,
    ) -> Result<(), CodegraphError> {
        let tx = self.conn.transaction().map_err(CodegraphError::Database)?;
        tx.execute("DELETE FROM nodes", [])
            .map_err(CodegraphError::Database)?;
        tx.execute("DELETE FROM edges", [])
            .map_err(CodegraphError::Database)?;
        tx.execute("DELETE FROM file_info", [])
            .map_err(CodegraphError::Database)?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO nodes(id,name,kind,file_path,start_line,end_line,signature,docstring,qualified_name,exported,lang,complexity,render)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                )
                .map_err(CodegraphError::Database)?;
            for s in symbols {
                stmt.execute(params![
                    s.id.0,
                    s.name,
                    s.kind.short(),
                    s.file,
                    s.line.map(|v| v as i64),
                    s.end_line.map(|v| v as i64),
                    s.signature,
                    s.doc,
                    s.qualified_name,
                    s.exported as i64,
                    s.lang,
                    s.complexity.map(|v| v as i64),
                    s.is_render as i64,
                ])
                .map_err(CodegraphError::Database)?;
            }
        }
        {
            let mut stmt = tx
                .prepare("INSERT INTO edges(id,source,target,kind,line,confidence,provenance) VALUES (?1,?2,?3,?4,?5,?6,?7)")
                .map_err(CodegraphError::Database)?;
            for r in relationships {
                stmt.execute(params![
                    r.id as i64,
                    r.source_id.0,
                    r.target_id.0,
                    r.kind.short(),
                    r.line.map(|v| v as i64),
                    r.confidence as f64,
                    r.provenance,
                ])
                .map_err(CodegraphError::Database)?;
            }
        }
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO file_info(path,lang,loc,symbol_count,status,complexity_sum,complexity_max,function_count,ignores)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                )
                .map_err(CodegraphError::Database)?;
            for f in files.values() {
                stmt.execute(params![
                    f.path,
                    f.lang,
                    f.loc as i64,
                    f.symbol_count as i64,
                    status_str(&f.status),
                    f.complexity_sum as i64,
                    f.complexity_max as i64,
                    f.function_count as i64,
                    serde_json::to_string(&f.ignores).unwrap_or_else(|_| "[]".to_string()),
                ])
                .map_err(CodegraphError::Database)?;
            }
        }
        tx.commit().map_err(CodegraphError::Database)?;
        self.set_meta("indexed_at", &unix_now().to_string())?;
        Ok(())
    }

    /// Load the cached graph, if any nodes were stored.
    pub fn load_graph(&self) -> Result<Option<CodeGraph>, CodegraphError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))
            .map_err(CodegraphError::Database)?;
        if count == 0 {
            return Ok(None);
        }
        let symbols = self.load_symbols()?;
        let relationships = self.load_edges()?;
        let files = self.load_files()?;
        let graph = CodeGraph::new_with_files(symbols, relationships, files)?;
        Ok(Some(graph))
    }

    fn load_symbols(&self) -> Result<Vec<Symbol>, CodegraphError> {
        let mut stmt = self.conn.prepare(
            "SELECT id,name,kind,file_path,start_line,end_line,signature,docstring,qualified_name,exported,lang,complexity,render FROM nodes",
        ).map_err(CodegraphError::Database)?;
        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let kind: String = row.get(2)?;
                let file: String = row.get(3)?;
                let mut sym = Symbol::new(SymbolId(id), name, SymbolKind::parse(&kind), file);
                sym.line = row.get::<_, Option<i64>>(4)?.map(|v| v as u32);
                sym.end_line = row.get::<_, Option<i64>>(5)?.map(|v| v as u32);
                sym.signature = row.get(6)?;
                sym.doc = row.get(7)?;
                sym.qualified_name = row.get(8)?;
                sym.exported = row.get::<_, i64>(9).unwrap_or(0) != 0;
                sym.lang = row.get(10)?;
                sym.complexity = row.get::<_, Option<i64>>(11)?.map(|v| v as u32);
                sym.is_render = row.get::<_, i64>(12).unwrap_or(0) != 0;
                Ok(sym)
            })
            .map_err(CodegraphError::Database)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(CodegraphError::Database)?);
        }
        Ok(out)
    }

    fn load_edges(&self) -> Result<Vec<Relationship>, CodegraphError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id,source,target,kind,line,confidence,provenance FROM edges ORDER BY id",
            )
            .map_err(CodegraphError::Database)?;
        let rows = stmt
            .query_map([], |row| {
                let kind: String = row.get(3)?;
                Ok(Relationship::indexed(
                    row.get::<_, i64>(0)? as u64,
                    SymbolId(row.get::<_, String>(1)?),
                    SymbolId(row.get::<_, String>(2)?),
                    RelationshipKind::parse(&kind),
                    row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                    row.get::<_, f64>(5)? as f32,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(CodegraphError::Database)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(CodegraphError::Database)?);
        }
        Ok(out)
    }

    fn load_files(&self) -> Result<std::collections::HashMap<String, FileInfo>, CodegraphError> {
        let mut stmt = self.conn.prepare(
            "SELECT path,lang,loc,symbol_count,status,complexity_sum,complexity_max,function_count,ignores FROM file_info",
        ).map_err(CodegraphError::Database)?;
        let rows = stmt
            .query_map([], |row| {
                let path: String = row.get(0)?;
                let status: String = row.get(4)?;
                Ok(FileInfo {
                    path,
                    lang: row.get(1)?,
                    loc: row.get::<_, i64>(2)? as usize,
                    symbol_count: row.get::<_, i64>(3)? as usize,
                    status: parse_status(&status),
                    complexity_sum: row.get::<_, i64>(5)? as u32,
                    complexity_max: row.get::<_, i64>(6)? as u32,
                    function_count: row.get::<_, i64>(7)? as usize,
                    ignores: row
                        .get::<_, Option<String>>(8)?
                        .and_then(|t| serde_json::from_str(&t).ok())
                        .unwrap_or_default(),
                })
            })
            .map_err(CodegraphError::Database)?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let f = row.map_err(CodegraphError::Database)?;
            out.insert(f.path.clone(), f);
        }
        Ok(out)
    }
}

fn status_str(status: &FileStatus) -> String {
    match status {
        FileStatus::Indexed => "indexed".into(),
        FileStatus::Cached => "cached".into(),
        FileStatus::SkippedLarge => "skipped_large".into(),
        FileStatus::Failed(e) => format!("failed:{e}"),
    }
}

fn parse_status(s: &str) -> FileStatus {
    match s {
        "indexed" => FileStatus::Indexed,
        "cached" => FileStatus::Cached,
        "skipped_large" => FileStatus::SkippedLarge,
        other => FileStatus::Failed(other.trim_start_matches("failed:").to_string()),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Default cache path for a project root.
pub fn cache_path_for(project_root: &Path) -> PathBuf {
    let id = crate::paths::project_root_id(project_root);
    if let Some(dirs) = crate::paths::app_dirs() {
        dirs.cache_dir()
            .join("projects")
            .join(id)
            .join("index.sqlite")
    } else {
        project_root.join(".codegraph-index.sqlite")
    }
}

/// Open the cache for `project_root` and load its graph, if present.
pub fn load_cached_graph(project_root: &Path) -> Option<CodeGraph> {
    let path = cache_path_for(project_root);
    if !path.exists() {
        return None;
    }
    let cache = IndexCache::open(path).ok()?;
    cache.load_graph().ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(name: &str) -> FileFacts {
        let mut f = FileFacts::default();
        f.symbols.push(super::super::facts::RawSymbol {
            kind: "function".into(),
            name: name.into(),
            qualified_name: name.into(),
            start_line: 1,
            end_line: 1,
            exported: true,
            parent: None,
        });
        f
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("index.sqlite")
    }

    #[test]
    fn facts_round_trip_by_hash() {
        let path = tmp("cg_cache_facts");
        let cache = IndexCache::open(&path).unwrap();
        cache
            .upsert_file(
                "a.ts",
                "h1",
                FileMeta {
                    lang: Some("typescript"),
                    loc: 3,
                    size: 10,
                    mtime: 0,
                },
                &facts("foo"),
            )
            .unwrap();
        assert!(cache.cached_facts("a.ts", "h1").unwrap().is_some());
        assert!(cache.cached_facts("a.ts", "h2").unwrap().is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn graph_round_trip() {
        let path = tmp("cg_cache_graph");
        let mut cache = IndexCache::open(&path).unwrap();
        let symbols = vec![Symbol::new(
            SymbolId("x".into()),
            "x",
            SymbolKind::Function,
            "a.ts",
        )];
        let edges = vec![Relationship::new(
            1,
            SymbolId("x".into()),
            SymbolId("x".into()),
            RelationshipKind::Calls,
        )];
        let mut files = std::collections::HashMap::new();
        files.insert(
            "a.ts".to_string(),
            FileInfo {
                path: "a.ts".into(),
                lang: Some("typescript".into()),
                loc: 5,
                symbol_count: 1,
                status: FileStatus::Indexed,
                complexity_sum: 2,
                complexity_max: 2,
                function_count: 1,
                ignores: Vec::new(),
            },
        );
        cache.replace_graph(&symbols, &edges, &files).unwrap();
        let loaded = cache.load_graph().unwrap().expect("graph");
        assert_eq!(loaded.symbol_count(), 1);
        assert_eq!(loaded.relationship_count(), 1);
        assert_eq!(loaded.file("a.ts").map(|f| f.loc), Some(5));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn prune_removes_missing() {
        let path = tmp("cg_cache_prune");
        let mut cache = IndexCache::open(&path).unwrap();
        cache
            .upsert_file(
                "a.ts",
                "h",
                FileMeta {
                    lang: None,
                    loc: 1,
                    size: 1,
                    mtime: 0,
                },
                &facts("a"),
            )
            .unwrap();
        cache
            .upsert_file(
                "b.ts",
                "h",
                FileMeta {
                    lang: None,
                    loc: 1,
                    size: 1,
                    mtime: 0,
                },
                &facts("b"),
            )
            .unwrap();
        let mut keep = HashSet::new();
        keep.insert("a.ts".to_string());
        cache.prune(&keep).unwrap();
        assert!(cache.cached_facts("a.ts", "h").unwrap().is_some());
        // b.ts is gone (hash lookup on the removed row returns None).
        assert!(cache.cached_facts("b.ts", "h").unwrap().is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
