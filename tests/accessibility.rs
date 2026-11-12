//! Accessibility tree tests.
//!
//! These render the real widgets through `egui_kittest` and query the AccessKit
//! tree, so a screen reader's view of the UI is what is being asserted — not a
//! helper function's return value.

use codegraph_desktop::graph_view::GraphView;
use codegraph_desktop::model::{Relationship, RelationshipKind, Symbol, SymbolId, SymbolKind};
use codegraph_desktop::ui::{chip, column_header, draw_graph, list_row, Touched};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;

#[test]
fn custom_painted_row_is_announced_as_a_labelled_button() {
    let mut harness = Harness::new_ui(|ui| {
        let clicked = list_row(
            ui,
            false,
            false,
            "fn",
            egui::Color32::from_rgb(120, 200, 255),
            "parse_input",
            "src/parser.rs:12",
        );
        assert!(!clicked);
    });
    harness.run();

    // Role and label both come from `widget_info`, which is the only way a
    // painter-drawn row reaches a screen reader.
    let row = harness.get_by_label("fn parse_input — src/parser.rs:12");
    assert_eq!(row.accesskit_node().role(), egui::accesskit::Role::Button);
}

#[test]
fn selected_row_reports_its_selected_state() {
    let mut harness = Harness::new_ui(|ui| {
        list_row(
            ui,
            true,
            false,
            "fn",
            egui::Color32::WHITE,
            "lexer",
            "src/lexer.rs:4",
        );
    });
    harness.run();
    let row = harness.get_by_label("fn lexer — src/lexer.rs:4");
    assert_eq!(row.accesskit_node().role(), egui::accesskit::Role::Button);
    // egui exposes `WidgetInfo::selected` as the toggled/checked state, which
    // screen readers announce as "selected".
    assert_eq!(
        row.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
}

#[test]
fn row_can_be_focused_and_activated_through_accesskit() {
    let activated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = activated.clone();

    let mut harness = Harness::new_ui(move |ui| {
        if list_row(
            ui,
            false,
            false,
            "fn",
            egui::Color32::WHITE,
            "run",
            "src/run.rs:1",
        ) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    harness.run();

    let row = harness.get_by_label("fn run — src/run.rs:1");
    row.focus();
    harness.run();

    let row = harness.get_by_label("fn run — src/run.rs:1");
    assert!(
        row.accesskit_node().is_focused(),
        "the row must be focusable for keyboard and screen-reader users"
    );

    row.click();
    harness.run();
    assert!(
        activated.load(std::sync::atomic::Ordering::Relaxed),
        "an AccessKit click must reach the row's click handler"
    );
}

#[test]
fn chip_text_is_in_the_accessibility_tree() {
    let mut harness = Harness::new_ui(|ui| {
        chip(ui, "244 matching", egui::Color32::WHITE);
    });
    harness.run();
    // Chips used to be painted rectangles; now they are real labels.
    harness.get_by_label("244 matching");
}

#[test]
fn column_header_is_announced_with_its_definition() {
    let mut harness = Harness::new_ui(|ui| {
        column_header(ui, "Ce", false);
    });
    harness.run();
    let header = harness.get_by_label_contains("Ce, column");
    assert_eq!(
        header.accesskit_node().role(),
        egui::accesskit::Role::Button
    );
    assert!(
        header
            .accesskit_node()
            .label()
            .unwrap_or_default()
            .contains("depends on"),
        "the tooltip text should reach the tree"
    );
}

#[test]
fn graph_nodes_are_focusable_labelled_buttons() {
    let symbols = vec![
        Symbol::new(SymbolId("a".into()), "api", SymbolKind::Module, "app/api"),
        Symbol::new(
            SymbolId("b".into()),
            "crm",
            SymbolKind::Module,
            "app/admin/crm",
        ),
    ];
    let edges = vec![Relationship::new(
        1,
        SymbolId("a".into()),
        SymbolId("b".into()),
        RelationshipKind::Imports,
    )];
    let view = GraphView::new(symbols, edges, None);
    let mut pan = egui::Vec2::ZERO;
    let mut zoom = 1.0;

    let mut harness = Harness::new_ui(move |ui| {
        let _ = draw_graph(ui, &view, &mut pan, &mut zoom, &Touched::default());
    });
    harness.run();

    let api = harness.get_by_label("api — app/api");
    assert_eq!(api.accesskit_node().role(), egui::accesskit::Role::Button);
    let crm = harness.get_by_label("crm — app/admin/crm");
    assert_eq!(crm.accesskit_node().role(), egui::accesskit::Role::Button);
    // The canvas itself carries a summary so the graph is not just "blank".
    harness.get_by_label_contains("Dependency graph, 2 nodes and 1 edges");
}
