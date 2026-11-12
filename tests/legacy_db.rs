//! Legacy `codegraph.db` compatibility.
//!
//! The app must keep opening databases produced by the separate CodeGraph
//! tool, including kind strings this build does not know.

use std::collections::HashMap;

use codegraph_desktop::model::{FileStatus, RelationshipKind, SymbolKind};
use codegraph_desktop::storage::Storage;
use codegraph_desktop::{CodeGraph, SqliteStorage};

fn legacy_db(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE nodes(
            id TEXT PRIMARY KEY, name TEXT, kind TEXT, file_path TEXT,
            start_line INTEGER, signature TEXT, docstring TEXT);
         CREATE TABLE edges(id INTEGER PRIMARY KEY, source TEXT, target TEXT, kind TEXT);
         INSERT INTO nodes VALUES ('1','parse','function','src/a.rs',3,NULL,NULL);
         INSERT INTO nodes VALUES ('2','Lexer','weird-kind','src/b.rs',7,'struct Lexer','doc');
         INSERT INTO edges VALUES (1,'1','2','references');
         INSERT INTO edges VALUES (2,'1','2','mystery-edge');",
    )
    .unwrap();
}

#[test]
fn legacy_database_opens_and_unknown_kinds_are_preserved() {
    let dir = std::env::temp_dir().join("cg_legacy_db");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("codegraph.db");
    legacy_db(&db);

    let storage = SqliteStorage::open(&db).expect("open legacy db");
    let symbols = storage.symbols().expect("symbols");
    let relationships = storage.relationships().expect("relationships");
    assert_eq!(symbols.len(), 2);
    assert_eq!(relationships.len(), 2);

    // Known kind maps; unknown kind is kept verbatim instead of failing.
    assert_eq!(symbols[0].kind, SymbolKind::Function);
    assert_eq!(symbols[1].kind, SymbolKind::Other("weird-kind".to_string()));
    assert_eq!(symbols[1].signature.as_deref(), Some("struct Lexer"));
    assert_eq!(symbols[1].line, Some(7));

    // Legacy edges default to full confidence and `legacy` provenance.
    assert_eq!(relationships[0].kind, RelationshipKind::References);
    assert_eq!(relationships[0].confidence, 1.0);
    assert_eq!(relationships[0].provenance, "legacy");
    assert_eq!(
        relationships[1].kind,
        RelationshipKind::Other("mystery-edge".to_string())
    );

    // Files are derived from nodes for legacy sources.
    let files = storage.files().expect("files");
    assert_eq!(files.len(), 2);
    assert_eq!(files["src/a.rs"].symbol_count, 1);

    // The graph builds and the UI-visible counts match.
    let mut with_files: HashMap<String, _> = HashMap::new();
    with_files.extend(files);
    let graph = CodeGraph::new_with_files(symbols, relationships, with_files).expect("graph");
    assert_eq!(graph.symbol_count(), 2);
    assert_eq!(graph.relationship_count(), 2);
    assert!(matches!(
        graph.file("src/b.rs").map(|f| f.status.clone()),
        Some(FileStatus::Indexed)
    ));

    let _ = std::fs::remove_dir_all(&dir);
}
