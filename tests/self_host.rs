//! Self-hosting test: index this repository and check the graph is sane.
//!
//! This exercises the Rust extractor and the `crate::` / `self::` / `super::`
//! resolver against real code, not a fixture.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use codegraph_desktop::config::ProjectConfig;
use codegraph_desktop::indexer::{index_project, IndexOutcome};
use codegraph_desktop::model::RelationshipKind;

fn index_self() -> IndexOutcome {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    index_project(
        &root,
        &ProjectConfig::default(),
        None,
        Arc::new(AtomicBool::new(false)),
    )
    .expect("self index")
}

#[test]
fn self_index_has_rust_symbols_and_crate_edges() {
    let outcome = index_self();
    let graph = &outcome.graph;

    assert!(
        outcome.summary.files >= 10,
        "expected many files, got {}",
        outcome.summary.files
    );
    assert!(
        graph.symbol_count() > 200,
        "expected many symbols, got {}",
        graph.symbol_count()
    );
    assert!(
        outcome.summary.failed == 0,
        "no file should fail to parse: {:?}",
        outcome.summary.notes
    );

    // The Rust extractor produced real symbols from real files.
    let rust_symbols = graph
        .symbols()
        .iter()
        .filter(|s| s.file.ends_with(".rs"))
        .count();
    assert!(rust_symbols > 100, "rust symbols: {rust_symbols}");

    // `src/main.rs` uses the library, so there is an edge from a main.rs symbol
    // to a lib module file.
    let import_edges: Vec<_> = graph
        .relationships()
        .iter()
        .filter(|r| r.kind == RelationshipKind::Imports)
        .filter(|r| {
            graph
                .symbol(r.source_id.clone())
                .is_some_and(|s| s.file == "src/main.rs")
        })
        .collect();
    assert!(
        !import_edges.is_empty(),
        "main.rs should import library modules"
    );

    // `src/analysis.rs` depends on `src/graph.rs`.
    let analysis_to_graph = graph.relationships().iter().any(|r| {
        let src = graph.symbol(r.source_id.clone()).map(|s| s.file.as_str());
        let dst = graph.symbol(r.target_id.clone()).map(|s| s.file.as_str());
        src == Some("src/analysis.rs") && dst == Some("src/graph.rs")
    });
    assert!(analysis_to_graph, "analysis.rs should depend on graph.rs");
}

#[test]
fn self_index_resolution_ratio_is_reasonable() {
    let outcome = index_self();
    let ratio = outcome.summary.resolved_ratio();
    // Measured at ~19% on this repo. Most refs here are method calls on
    // standard-library and third-party receivers (`.iter()`, `.unwrap()`,
    // `.clone()`), which this indexer deliberately does not guess at. The
    // floor only catches a real regression, it is not a quality target.
    assert!(
        ratio > 0.12,
        "refs resolved {:.0}% ({} / {}) is below the floor",
        ratio * 100.0,
        outcome.summary.refs_resolved,
        outcome.summary.refs_total
    );
    // The summary says so out loud instead of pretending the graph is complete.
    assert!(
        outcome
            .summary
            .notes
            .iter()
            .any(|n| n.contains("references resolved")),
        "expected an index-quality note, got {:?}",
        outcome.summary.notes
    );
}
