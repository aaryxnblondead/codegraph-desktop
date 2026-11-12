//! Live-sync integration: a file edit must be seen, re-indexed incrementally,
//! and reported as "touched" for the highlight layer.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use codegraph_desktop::config::ProjectConfig;
use codegraph_desktop::indexer::cache::IndexCache;
use codegraph_desktop::indexer::index_project_cached;
use codegraph_desktop::ui::Touched;
use codegraph_desktop::watch::ProjectWatcher;

fn write(dir: &std::path::Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn index(
    dir: &std::path::Path,
    cache: &std::path::Path,
) -> codegraph_desktop::indexer::IndexOutcome {
    index_project_cached(
        dir,
        &ProjectConfig::default(),
        None,
        Arc::new(AtomicBool::new(false)),
        Some(cache),
    )
    .expect("index")
}

#[test]
fn edit_is_watched_reindexed_and_marked_live() {
    let dir = std::env::temp_dir().join("cg_live_project");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "src/a.ts", "export function a() { return 1; }\n");
    write(&dir, "src/b.ts", "export function b() { return 2; }\n");
    let cache = dir.join("index.sqlite");

    let first = index(&dir, &cache);
    let edges_before = first.graph.relationship_count();

    // Watch, then make an edit that adds a dependency.
    let watcher = ProjectWatcher::start(&dir).expect("watcher");
    write(
        &dir,
        "src/b.ts",
        "import { a } from './a';\nexport function b() { return a(); }\n",
    );

    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline && !seen.iter().any(|p| p == "src/b.ts") {
        seen.extend(watcher.try_drain());
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        seen.iter().any(|p| p == "src/b.ts"),
        "watcher did not report src/b.ts: {seen:?}"
    );

    let second = index(&dir, &cache);
    assert_eq!(
        second.summary.parsed, 1,
        "only the edited file should be re-parsed (cached: {})",
        second.summary.cached
    );
    assert_eq!(second.summary.cached, 1, "the untouched file is reused");
    assert!(
        second.graph.relationship_count() > edges_before,
        "the new import should add edges: {} -> {}",
        edges_before,
        second.graph.relationship_count()
    );

    // The highlight layer lights up the file and its module.
    let touched = Touched {
        paths: vec!["src/b.ts".to_string()],
    };
    assert!(touched.contains_file("src/b.ts"));
    assert!(touched.matches("src/b.ts"), "file view");
    assert!(touched.matches("src"), "module view");
    assert!(
        !touched.matches("src/a.ts"),
        "untouched neighbours stay dark"
    );
    assert!(
        !touched.matches("srcx"),
        "prefix must not match on partial names"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persisted_graph_matches_the_live_reindex() {
    let dir = std::env::temp_dir().join("cg_live_cache");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "m.ts", "export function m() { return 1; }\n");
    let cache_path = dir.join("index.sqlite");

    let first = index(&dir, &cache_path);
    write(&dir, "n.ts", "export function n() { return 2; }\n");
    let second = index(&dir, &cache_path);
    assert_eq!(second.summary.parsed, 1);
    assert_eq!(second.summary.cached, 1);

    let cached = IndexCache::open(&cache_path)
        .unwrap()
        .load_graph()
        .unwrap()
        .expect("cached graph");
    assert_eq!(cached.symbol_count(), second.graph.symbol_count());
    assert_eq!(
        cached.relationship_count(),
        second.graph.relationship_count()
    );
    assert!(cached.file("n.ts").is_some(), "new file is in the cache");
    assert!(first.graph.file("n.ts").is_none());

    let _ = std::fs::remove_dir_all(&dir);
}
