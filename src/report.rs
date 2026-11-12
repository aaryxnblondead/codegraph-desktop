//! Deterministic report export: JSON and Markdown.
//!
//! Same input always produces byte-identical output, so reports diff cleanly
//! between runs and can be committed or compared in CI.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::graph::CodeGraph;
use crate::metrics::MetricsBundle;
use crate::structure::CutEdge;
use crate::warnings::{Severity, Warning};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub files: usize,
    pub symbols: usize,
    pub edges: usize,
    pub refs_total: usize,
    pub refs_resolved: usize,
    pub resolved_ratio: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleRow {
    pub module: String,
    pub ca: usize,
    pub ce: usize,
    pub instability: Option<f32>,
    pub abstractness: Option<f32>,
    pub distance: Option<f32>,
    pub cohesion_components: usize,
    pub cohesion_largest_ratio: f32,
    pub complexity_sum: u32,
    pub files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleRow {
    pub files: Vec<String>,
    pub suggested_cuts: Vec<CutRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CutRow {
    pub from: String,
    pub to: String,
    pub weight: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningRow {
    pub id: String,
    pub rule: String,
    pub severity: String,
    pub message: String,
    pub suggestion: String,
    pub subjects: Vec<String>,
    /// Exact targets: `file::symbol:line–end` where known.
    #[serde(default)]
    pub locations: Vec<crate::warnings::Location>,
}

/// New vs fixed findings relative to a saved baseline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeltaRow {
    pub new: Vec<WarningRow>,
    pub fixed: Vec<FixedRow>,
    pub unchanged: usize,
    pub error_delta: i64,
    pub warn_delta: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FixedRow {
    pub id: String,
    pub rule: String,
    pub severity: String,
    pub message: String,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub summary: Summary,
    pub modules: Vec<ModuleRow>,
    pub warnings: Vec<WarningRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<DeltaRow>,
    pub cycles: Vec<CycleRow>,
    /// Files with no resolved incoming/outgoing edges — index-quality hint.
    pub unresolved_files: Vec<String>,
}

impl Report {
    /// Build a report. Ordering is normalized so output is deterministic.
    pub fn build(graph: &CodeGraph, metrics: &MetricsBundle, warnings: &[Warning]) -> Self {
        Self::build_with_delta(graph, metrics, warnings, None)
    }

    /// Build a report, optionally including a baseline diff.
    pub fn build_with_delta(
        graph: &CodeGraph,
        metrics: &MetricsBundle,
        warnings: &[Warning],
        delta: Option<&crate::baseline::BaselineDiff>,
    ) -> Self {
        let files_with_edges: std::collections::HashSet<&str> = graph
            .symbols()
            .iter()
            .filter(|s| {
                !graph
                    .relationships_for(s.id.clone(), crate::model::EdgeDirection::Outgoing)
                    .is_empty()
                    || !graph
                        .relationships_for(s.id.clone(), crate::model::EdgeDirection::Incoming)
                        .is_empty()
            })
            .map(|s| s.file.as_str())
            .collect();

        let mut unresolved_files: Vec<String> = graph
            .files()
            .keys()
            .filter(|f| !files_with_edges.contains(f.as_str()))
            .cloned()
            .collect();
        unresolved_files.sort();

        let cross = crate::warnings::cross_edges(graph, &crate::config::ProjectConfig::default());
        let cross_list: Vec<(String, String)> = cross.keys().cloned().collect();
        let cycles = crate::analysis::find_file_cycles(graph, 0.5)
            .into_iter()
            .map(|files| {
                let cuts = crate::structure::break_cycle(&files, &cross_list, &cross);
                CycleRow {
                    files,
                    suggested_cuts: cuts.into_iter().map(CutRow::from).collect(),
                }
            })
            .collect();

        let mut modules: Vec<ModuleRow> = metrics
            .modules
            .iter()
            .map(|m| ModuleRow {
                module: m.unit.clone(),
                ca: m.ca,
                ce: m.ce,
                instability: m.instability,
                abstractness: m.abstractness,
                distance: m.distance,
                cohesion_components: m.cohesion_components,
                cohesion_largest_ratio: m.cohesion_largest_ratio,
                complexity_sum: m.complexity_sum,
                files: graph
                    .files()
                    .keys()
                    .filter(|f| {
                        // Count files by the module label stored on the metric.
                        m.unit == crate::graph_view::module_key(f)
                    })
                    .count(),
            })
            .collect();
        modules.sort_by(|a, b| a.module.cmp(&b.module));

        Report {
            summary: Summary {
                files: graph.files().len(),
                symbols: graph.symbol_count(),
                edges: graph.relationship_count(),
                refs_total: 0,
                refs_resolved: 0,
                resolved_ratio: 0.0,
            },
            modules,
            warnings: warnings.iter().map(WarningRow::from).collect(),
            delta: delta.map(|d| DeltaRow {
                new: d
                    .new_findings
                    .iter()
                    .map(|e| WarningRow {
                        id: e.id.clone(),
                        rule: e.rule.clone(),
                        severity: e.severity.clone(),
                        message: e.message.clone(),
                        suggestion: String::new(),
                        subjects: Vec::new(),
                        locations: e
                            .location
                            .clone()
                            .map(|l| crate::warnings::Location {
                                file: l,
                                symbol: None,
                                line: None,
                                end_line: None,
                            })
                            .into_iter()
                            .collect(),
                    })
                    .collect(),
                fixed: d
                    .fixed_findings
                    .iter()
                    .map(|e| FixedRow {
                        id: e.id.clone(),
                        rule: e.rule.clone(),
                        severity: e.severity.clone(),
                        message: e.message.clone(),
                        location: e.location.clone(),
                    })
                    .collect(),
                unchanged: d.unchanged,
                error_delta: d.error_delta,
                warn_delta: d.warn_delta,
            }),
            cycles,
            unresolved_files,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# CodeGraph report\n\n");
        out.push_str(&format!(
            "{} files · {} symbols · {} edges\n\n",
            self.summary.files, self.summary.symbols, self.summary.edges
        ));

        out.push_str("## Modules\n\n");
        out.push_str("| Module | Files | Ca | Ce | I | A | D | Cohesion | Complexity |\n");
        out.push_str("|---|---|---|---|---|---|---|---|---|\n");
        for m in &self.modules {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {}/{} | {} |\n",
                m.module,
                m.files,
                m.ca,
                m.ce,
                opt(m.instability),
                opt(m.abstractness),
                opt(m.distance),
                m.cohesion_components,
                m.cohesion_largest_ratio,
                m.complexity_sum
            ));
        }

        if let Some(delta) = &self.delta {
            out.push_str(&format!(
                "\n## Since baseline\n\n{} new · {} fixed · {} unchanged (errors {:+}, warnings {:+})\n\n",
                delta.new.len(),
                delta.fixed.len(),
                delta.unchanged,
                delta.error_delta,
                delta.warn_delta
            ));
            for n in delta.new.iter().take(10) {
                out.push_str(&format!(
                    "- NEW **{}** `{}` — {}\n",
                    n.severity, n.rule, n.message
                ));
            }
            for f in delta.fixed.iter().take(10) {
                out.push_str(&format!("- FIXED `{}` — {}\n", f.rule, f.message));
            }
        }

        out.push_str("\n## Warnings\n\n");
        if self.warnings.is_empty() {
            out.push_str("No warnings.\n");
        }
        for w in &self.warnings {
            out.push_str(&format!(
                "- **{}** `{}` — {}\n  - {}\n",
                w.severity, w.rule, w.message, w.suggestion
            ));
        }

        out.push_str("\n## Cycles\n\n");
        if self.cycles.is_empty() {
            out.push_str("No cycles.\n");
        }
        for c in &self.cycles {
            out.push_str(&format!("- {} files\n", c.files.len()));
            for cut in &c.suggested_cuts {
                out.push_str(&format!(
                    "  - cut `{} -> {}` (weight {})\n",
                    cut.from, cut.to, cut.weight
                ));
            }
        }

        out.push_str("\n## Index quality\n\n");
        out.push_str(&format!(
            "{} files have no resolved edges.\n",
            self.unresolved_files.len()
        ));
        out
    }

    /// Warning counts by severity.
    pub fn severity_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for w in &self.warnings {
            *counts.entry(w.severity.clone()).or_insert(0) += 1;
        }
        counts
    }
}

fn opt(value: Option<f32>) -> String {
    match value {
        Some(v) => format!("{v:.2}"),
        None => "–".to_string(),
    }
}

impl From<&Warning> for WarningRow {
    fn from(w: &Warning) -> Self {
        Self {
            id: w.id.clone(),
            rule: w.rule.clone(),
            severity: match w.severity {
                Severity::Error => "error".into(),
                Severity::Warn => "warn".into(),
                Severity::Info => "info".into(),
            },
            message: w.message.clone(),
            suggestion: w.suggestion.clone(),
            subjects: w.subjects.clone(),
            locations: w.locations.clone(),
        }
    }
}

impl From<CutEdge> for CutRow {
    fn from(c: CutEdge) -> Self {
        Self {
            from: c.from,
            to: c.to,
            weight: c.weight,
        }
    }
}

// ---- clipboard text -----------------------------------------------------
//
// Every analysis pane can be copied as plain text. The formatting lives here
// (not in the UI) so it is unit-tested and stable enough to paste into an
// issue, a chat, or a commit message.

/// One warning as text:
/// `[ERROR] complex_function — message` then `    → suggestion`.
pub fn warning_text(w: &Warning) -> String {
    let mut out = format!(
        "[{}] {} — {}",
        match w.severity {
            Severity::Error => "ERROR",
            Severity::Warn => "WARN",
            Severity::Info => "INFO",
        },
        w.rule,
        w.message
    );
    if let Some(loc) = w.locations.first() {
        out.push_str(&format!("\n    at {}", loc.display()));
    }
    if !w.suggestion.is_empty() {
        out.push_str(&format!("\n    → {}", w.suggestion));
    }
    out
}

/// A whole warning list, with a count header and blank lines between entries.
pub fn warnings_text(warnings: &[Warning]) -> String {
    if warnings.is_empty() {
        return "CodeGraph: no warnings.".to_string();
    }
    let (mut e, mut wr, mut i) = (0, 0, 0);
    for w in warnings {
        match w.severity {
            Severity::Error => e += 1,
            Severity::Warn => wr += 1,
            Severity::Info => i += 1,
        }
    }
    let mut out = format!(
        "CodeGraph warnings: {} error · {} warn · {} info\n\n",
        e, wr, i
    );
    for w in warnings {
        out.push_str(&warning_text(w));
        out.push_str("\n\n");
    }
    out.trim_end().to_string()
}

/// A cycle group with its suggested cuts.
pub fn cycle_text(files: &[String], cuts: &[CutEdge]) -> String {
    let mut out = format!("Dependency cycle ({} files):\n", files.len());
    for f in files {
        out.push_str(&format!("  {f}\n"));
    }
    if cuts.is_empty() {
        out.push_str("No reliable cut: all edges in this group are low-confidence.\n");
    } else {
        out.push_str("Suggested cuts (cheapest first):\n");
        for c in cuts {
            out.push_str(&format!(
                "  {} -> {}  (weight {})\n",
                c.from, c.to, c.weight
            ));
        }
    }
    out.trim_end().to_string()
}

/// Impact summary for one symbol.
pub fn impact_text(
    symbol: &str,
    dependents: &[(usize, usize)],
    dependencies: &[(usize, usize)],
) -> String {
    let mut out = format!("Impact of {symbol}\n");
    out.push_str(&format!(
        "Dependents (what breaks): {}\n",
        dependents.iter().map(|(_, n)| n).sum::<usize>()
    ));
    for (depth, count) in dependents {
        out.push_str(&format!("  depth {depth}: {count}\n"));
    }
    out.push_str(&format!(
        "Dependencies (what it needs): {}\n",
        dependencies.iter().map(|(_, n)| n).sum::<usize>()
    ));
    for (depth, count) in dependencies {
        out.push_str(&format!("  depth {depth}: {count}\n"));
    }
    out.trim_end().to_string()
}

/// One module table row.
pub fn module_text(m: &ModuleRow) -> String {
    format!(
        "{} · {} files · Ca {} Ce {} I {} A {} D {} · cohesion {}/{} · cx {}",
        m.module,
        m.files,
        m.ca,
        m.ce,
        opt(m.instability),
        opt(m.abstractness),
        opt(m.distance),
        m.cohesion_components,
        m.cohesion_largest_ratio,
        m.complexity_sum
    )
}

/// A symbol reference to paste into a chat: `path:line  name`.
pub fn symbol_ref(file: &str, line: Option<u32>, name: &str) -> String {
    match line {
        Some(line) => format!("{file}:{line}  {name}"),
        None => format!("{file}  {name}"),
    }
}

/// Header line describing what is on screen.
pub fn context_text(
    project: &str,
    branch: Option<&str>,
    head: Option<&str>,
    counts: (usize, usize, usize),
) -> String {
    let branch = branch.unwrap_or("detached");
    let head = head.map(|h| &h[..h.len().min(8)]).unwrap_or("?");
    format!(
        "{project} @ {branch} ({head}) · {} files · {} symbols · {} edges",
        counts.0, counts.1, counts.2
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::metrics;
    use crate::model::{
        FileInfo, FileStatus, Relationship, RelationshipKind, Symbol, SymbolId, SymbolKind,
    };
    use crate::modules::ModuleAssigner;
    use crate::warnings;

    fn graph() -> CodeGraph {
        let mut files = std::collections::HashMap::new();
        for f in ["a.ts", "b.ts"] {
            files.insert(
                f.to_string(),
                FileInfo {
                    path: f.into(),
                    lang: Some("typescript".into()),
                    loc: 10,
                    symbol_count: 1,
                    status: FileStatus::Indexed,
                    complexity_sum: 1,
                    complexity_max: 1,
                    function_count: 1,
                    ignores: Vec::new(),
                },
            );
        }
        CodeGraph::new_with_files(
            vec![
                Symbol::new(SymbolId("a".into()), "a", SymbolKind::Function, "a.ts"),
                Symbol::new(SymbolId("b".into()), "b", SymbolKind::Function, "b.ts"),
            ],
            vec![
                Relationship::new(
                    1,
                    SymbolId("a".into()),
                    SymbolId("b".into()),
                    RelationshipKind::Imports,
                ),
                Relationship::new(
                    2,
                    SymbolId("b".into()),
                    SymbolId("a".into()),
                    RelationshipKind::Imports,
                ),
            ],
            files,
        )
        .unwrap()
    }

    fn report() -> Report {
        let g = graph();
        let cfg = ProjectConfig::default();
        let assigner = ModuleAssigner::new(&[]);
        let metrics = metrics::compute(&g, &assigner, &cfg);
        let warnings = warnings::run_warnings(&g, &assigner, &metrics, &cfg);
        Report::build(&g, &metrics, &warnings)
    }

    #[test]
    fn warning_text_is_copy_pasteable() {
        let w = Warning {
            id: "abc".into(),
            severity: Severity::Error,
            rule: "cycle".into(),
            message: "2 files form a dependency cycle".into(),
            subjects: vec!["a.ts".into(), "b.ts".into()],
            count: 2,
            metrics: std::collections::BTreeMap::new(),
            explanation: "loop".into(),
            suggestion: "Cut a.ts -> b.ts.".into(),
            locations: Vec::new(),
        };
        let text = warning_text(&w);
        assert_eq!(
            text,
            "[ERROR] cycle — 2 files form a dependency cycle\n    → Cut a.ts -> b.ts."
        );
        let all = warnings_text(&[w]);
        assert!(all.starts_with("CodeGraph warnings: 1 error · 0 warn · 0 info"));
        assert!(all.contains("[ERROR] cycle"));
    }

    #[test]
    fn cycle_and_impact_text_are_readable() {
        let files = vec!["a.ts".to_string(), "b.ts".to_string()];
        let cuts = vec![CutEdge {
            from: "b.ts".into(),
            to: "a.ts".into(),
            weight: 2,
        }];
        let text = cycle_text(&files, &cuts);
        assert!(text.contains("Dependency cycle (2 files)"));
        assert!(text.contains("b.ts -> a.ts  (weight 2)"));

        let impact = impact_text("parse", &[(1, 3), (2, 1)], &[(1, 4)]);
        assert!(impact.contains("Dependents (what breaks): 4"));
        assert!(impact.contains("depth 1: 3"));
        assert!(impact.contains("Dependencies (what it needs): 4"));
    }

    #[test]
    fn symbol_ref_and_context() {
        assert_eq!(symbol_ref("src/a.rs", Some(12), "run"), "src/a.rs:12  run");
        assert_eq!(symbol_ref("src/a.rs", None, "run"), "src/a.rs  run");
        let ctx = context_text("/p", Some("main"), Some("0123456789abcdef"), (10, 20, 30));
        assert_eq!(
            ctx,
            "/p @ main (01234567) · 10 files · 20 symbols · 30 edges"
        );
        let ctx = context_text("/p", None, None, (1, 2, 3));
        assert!(ctx.contains("detached"));
    }

    #[test]
    fn json_is_deterministic() {
        let a = report().to_json();
        let b = report().to_json();
        assert_eq!(a, b);
        assert!(a.contains("\"modules\""));
    }

    #[test]
    fn markdown_is_deterministic_and_has_sections() {
        let a = report().to_markdown();
        assert_eq!(a, report().to_markdown());
        for section in ["## Modules", "## Warnings", "## Cycles", "## Index quality"] {
            assert!(a.contains(section), "missing {section}");
        }
    }

    #[test]
    fn cycle_suggestions_are_reported() {
        let r = report();
        assert_eq!(r.cycles.len(), 1);
        assert_eq!(r.cycles[0].suggested_cuts.len(), 1);
        assert!(r
            .warnings
            .iter()
            .any(|w| w.rule == "cycle" && w.suggestion.contains("Cut")));
    }
}
