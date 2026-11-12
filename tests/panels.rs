//! The data behind the Modules and Cycles tabs, on a real indexed project.
//!
//! The panels are thin formatting over these functions, so this checks the
//! numbers a user actually sees: module coupling/cohesion and cycle cuts.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use codegraph_desktop::config::{ModuleDefConfig, ProjectConfig};
use codegraph_desktop::indexer::index_project;
use codegraph_desktop::metrics;
use codegraph_desktop::modules::{self, ModuleAssigner};
use codegraph_desktop::structure;
use codegraph_desktop::warnings;

fn write(dir: &std::path::Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

#[test]
fn module_metrics_and_cycle_cuts_are_populated() {
    let dir = std::env::temp_dir().join("cg_panels");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Two modules: `ui` depends on `domain`, and `domain` illegally depends
    // back on `ui`, which also creates a file-level cycle.
    write(
        &dir,
        "ui/view.ts",
        "import { model } from '../domain/model';\nexport function view() { return model(); }\n",
    );
    write(
        &dir,
        "domain/model.ts",
        "import { view } from '../ui/view';\nexport function model() { return view(); }\n",
    );

    let config = ProjectConfig {
        modules: vec![
            ModuleDefConfig {
                name: "UI".into(),
                paths: vec!["ui/**".into()],
                allowed_deps: Some(vec!["Domain".into()]),
            },
            ModuleDefConfig {
                name: "Domain".into(),
                paths: vec!["domain/**".into()],
                allowed_deps: Some(vec![]),
            },
        ],
        ..Default::default()
    };

    let outcome =
        index_project(&dir, &config, None, Arc::new(AtomicBool::new(false))).expect("index");
    let graph = &outcome.graph;

    // --- Modules tab data ---
    let assigner = ModuleAssigner::new(&config.modules);
    assert_eq!(assigner.module_of("ui/view.ts"), "UI");
    assert_eq!(assigner.module_of("domain/model.ts"), "Domain");

    let metrics = metrics::compute(graph, &assigner, &config);
    let ui = metrics
        .modules
        .iter()
        .find(|m| m.unit == "UI")
        .expect("UI module metrics");
    assert!(metrics.modules.iter().any(|m| m.unit == "Domain"));
    assert_eq!(ui.ce, 1, "UI depends on Domain");
    assert_eq!(ui.ca, 1, "Domain depends on UI");
    assert_eq!(ui.file_count, 1);
    assert!(ui.instability.is_some());
    assert_eq!(ui.cohesion_components, 1);

    // --- Warnings: the layer rule fires for Domain -> UI ---
    let found = warnings::run_warnings(graph, &assigner, &metrics, &config);
    let violation = found
        .iter()
        .find(|w| w.rule == "layer_violation")
        .expect("layer violation warning");
    assert!(
        violation.message.contains("Domain") && violation.message.contains("UI"),
        "{}",
        violation.message
    );

    // --- Cycles tab data: one group, cuts verified to break it ---
    let cross = warnings::cross_edges(graph, &config);
    let cross_list: Vec<(String, String)> = cross.keys().cloned().collect();
    let cycles = codegraph_desktop::analysis::find_file_cycles(graph, 0.5);
    assert_eq!(cycles.len(), 1, "expected one cycle group: {cycles:?}");
    let cuts = structure::break_cycle(&cycles[0], &cross_list, &cross);
    assert_eq!(cuts.len(), 1, "one edge breaks a two-file cycle: {cuts:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn starter_modules_make_every_file_reachable() {
    let defs = modules::starter_modules(&["src".to_string(), "tests".to_string()]);
    let assigner = ModuleAssigner::new(&defs);
    assert_eq!(assigner.module_of("src/main.rs"), "src");
    assert_eq!(assigner.module_of("tests/cli.rs"), "tests");
    assert_eq!(assigner.module_of("Cargo.toml"), "Root files");
    assert_eq!(assigner.module_of("docs/METRICS.md"), "Unassigned");
}
