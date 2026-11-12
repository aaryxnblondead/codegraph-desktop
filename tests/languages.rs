//! End-to-end language tests: write a tiny project, index it, check the graph.
//!
//! These go through the real extractor and resolver, so they catch wiring bugs
//! that unit tests on facts alone cannot.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use codegraph_desktop::config::ProjectConfig;
use codegraph_desktop::graph::CodeGraph;
use codegraph_desktop::indexer::index_project;
use codegraph_desktop::model::RelationshipKind;

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn index(dir: &Path) -> CodeGraph {
    let outcome = index_project(
        dir,
        &ProjectConfig::default(),
        None,
        Arc::new(AtomicBool::new(false)),
    )
    .expect("index");
    assert_eq!(
        outcome.summary.failed, 0,
        "no file should fail: {:?}",
        outcome.summary.notes
    );
    outcome.graph
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn files_linked(graph: &CodeGraph, from: &str, to: &str) -> bool {
    graph.relationships().iter().any(|r| {
        let src = graph.symbol(r.source_id.clone()).map(|s| s.file.as_str());
        let dst = graph.symbol(r.target_id.clone()).map(|s| s.file.as_str());
        src == Some(from) && dst == Some(to)
    })
}

#[test]
fn python_package_indexes_with_relative_and_absolute_imports() {
    let dir = temp_dir("cg_py_project");
    write(&dir, "pkg/__init__.py", "");
    write(
        &dir,
        "pkg/models.py",
        "class Item:\n    def __init__(self, name):\n        self.name = name\n",
    );
    write(
        &dir,
        "pkg/service.py",
        "from .models import Item\n\n\ndef make(name):\n    return Item(name)\n",
    );
    write(
        &dir,
        "pkg/app.py",
        "from pkg.service import make\n\n\ndef main():\n    return make('x')\n",
    );

    let graph = index(&dir);
    assert!(
        graph.symbol_count() >= 8,
        "symbols: {}",
        graph.symbol_count()
    );
    assert!(
        files_linked(&graph, "pkg/service.py", "pkg/models.py"),
        "relative import should link service.py -> models.py"
    );
    assert!(
        files_linked(&graph, "pkg/app.py", "pkg/service.py"),
        "absolute import should link app.py -> service.py"
    );
    assert!(graph
        .symbols()
        .iter()
        .any(|s| s.qualified_name == "Item::__init__" && s.kind.short() == "method"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rust_binary_indexes_crate_and_super_paths() {
    let dir = temp_dir("cg_rust_project");
    write(
        &dir,
        "Cargo.toml",
        "[package]\nname = \"mini-crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&dir, "src/lib.rs", "pub mod core;\npub mod util;\n");
    write(&dir, "src/core.rs", "pub fn work() -> u32 { 1 }\n");
    write(&dir, "src/util/mod.rs", "pub mod helper;\n");
    write(
        &dir,
        "src/util/helper.rs",
        "pub fn help() -> u32 { super::super::core::work() }\n",
    );
    write(
        &dir,
        "src/main.rs",
        "use mini_crate::core::work;\n\nfn main() {\n    let _ = work();\n}\n",
    );

    let graph = index(&dir);
    assert!(
        files_linked(&graph, "src/util/helper.rs", "src/core.rs"),
        "super::super::core::work should link helper.rs -> core.rs"
    );
    assert!(
        files_linked(&graph, "src/main.rs", "src/core.rs"),
        "package-name path should link main.rs -> core.rs"
    );
    assert!(
        graph.relationships().iter().any(|r| {
            r.kind == RelationshipKind::Calls
                && graph
                    .symbol(r.target_id.clone())
                    .is_some_and(|s| s.name == "work" && s.file == "src/core.rs")
        }),
        "the call to `work` should resolve to core.rs"
    );
    assert!(graph.relationships().iter().any(|r| {
        r.kind == RelationshipKind::Calls
            && r.provenance == "import-binding"
            && graph
                .symbol(r.target_id.clone())
                .is_some_and(|s| s.name == "work")
    }));

    let _ = std::fs::remove_dir_all(&dir);
}
