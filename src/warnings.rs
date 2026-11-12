//! Warning engine: rule-based findings over the indexed graph.
//!
//! Design rules learned from real reports:
//! - A finding must say **where**: `file::symbol:line–end`, not just a file.
//! - Thresholds must be calibrated: a rule that fires on most of a codebase is
//!   measuring itself, not the code.
//! - Index-quality numbers (unresolved references) are not dead-code findings.
//! - Suppression is explicit: committed config rules and inline comments.

use std::collections::{BTreeMap, HashMap, HashSet};

use globset::{Glob, GlobMatcher};

use crate::config::ProjectConfig;
use crate::graph::CodeGraph;
use crate::metrics::MetricsBundle;
use crate::model::SymbolKind;
use crate::modules::ModuleAssigner;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warn => "WARN",
            Severity::Error => "ERROR",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Where a finding points. `symbol` and the line range are filled whenever the
/// rule knows the exact function, so a reader can jump straight to it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Location {
    pub file: String,
    pub symbol: Option<String>,
    pub line: Option<u32>,
    pub end_line: Option<u32>,
}

impl Location {
    pub fn file(file: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            ..Default::default()
        }
    }

    pub fn symbol(
        file: impl Into<String>,
        name: impl Into<String>,
        line: u32,
        end_line: u32,
    ) -> Self {
        Self {
            file: file.into(),
            symbol: Some(name.into()),
            line: Some(line),
            end_line: Some(end_line),
        }
    }

    /// `app/admin/actions.ts::applyRefund:412–479`
    pub fn display(&self) -> String {
        let mut out = self.file.clone();
        if let Some(sym) = &self.symbol {
            out.push_str("::");
            out.push_str(sym);
        }
        match (self.line, self.end_line) {
            (Some(a), Some(b)) if b > a => out.push_str(&format!(":{a}\u{2013}{b}")),
            (Some(a), _) => out.push_str(&format!(":{a}")),
            _ => {}
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct Warning {
    pub id: String,
    pub severity: Severity,
    pub rule: String,
    pub message: String,
    pub subjects: Vec<String>,
    pub count: usize,
    pub metrics: BTreeMap<String, f64>,
    pub explanation: String,
    pub suggestion: String,
    pub locations: Vec<Location>,
}

/// Stable warning id: 16 hex chars of blake3(rule + sorted subjects).
pub fn warning_id(rule: &str, subjects: &[String]) -> String {
    let mut sorted = subjects.to_vec();
    sorted.sort();
    let mut hasher = blake3::Hasher::new();
    hasher.update(rule.as_bytes());
    hasher.update(b"\0");
    for s in &sorted {
        hasher.update(s.as_bytes());
        hasher.update(b"\0");
    }
    hasher.finalize().to_hex().chars().take(16).collect()
}

/// How many suppressed findings were dropped, for the single summary note.
#[derive(Debug, Clone, Default)]
pub struct Suppressed {
    pub config: usize,
    pub inline: usize,
}

/// Run all rules. Output is sorted error → warn → info, then by rule.
pub fn run_warnings(
    graph: &CodeGraph,
    assigner: &ModuleAssigner,
    metrics: &MetricsBundle,
    config: &ProjectConfig,
) -> Vec<Warning> {
    run_warnings_reporting(graph, assigner, metrics, config).0
}

/// Same as [`run_warnings`], plus suppression counts.
pub fn run_warnings_reporting(
    graph: &CodeGraph,
    assigner: &ModuleAssigner,
    metrics: &MetricsBundle,
    config: &ProjectConfig,
) -> (Vec<Warning>, Suppressed) {
    let mut out: Vec<Warning> = Vec::new();
    let cross = cross_edges(graph, config);
    let cross_list: Vec<(String, String)> = cross.keys().cloned().collect();

    // Index the graph once: symbols per file and per-file degree.
    let mut symbols_by_file: HashMap<&str, Vec<&crate::model::Symbol>> = HashMap::new();
    for s in graph.symbols() {
        symbols_by_file.entry(s.file.as_str()).or_default().push(s);
    }
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for r in graph.relationships() {
        if r.kind.is_structural() {
            continue;
        }
        if let Some(s) = graph.symbol(r.source_id.clone()) {
            *degree.entry(s.file.as_str()).or_default() += 1;
        }
        if let Some(t) = graph.symbol(r.target_id.clone()) {
            *degree.entry(t.file.as_str()).or_default() += 1;
        }
    }

    // ---- cycles: exact files and the cheapest cuts -----------------------
    for cycle in crate::analysis::find_file_cycles(graph, config.min_confidence) {
        let cuts = crate::structure::break_cycle(&cycle, &cross_list, &cross);
        let listed: Vec<String> = cuts
            .iter()
            .take(3)
            .map(|c| format!("{} -> {}", c.from, c.to))
            .collect();
        let cut_text = if cuts.is_empty() {
            " All edges in the group are low-confidence; confirm by hand first.".to_string()
        } else {
            format!(
                " Cut {} edge(s): {}{}.",
                cuts.len(),
                listed.join(", "),
                if cuts.len() > 3 { ", …" } else { "" }
            )
        };
        let mut m = BTreeMap::new();
        m.insert("files".to_string(), cycle.len() as f64);
        out.push(Warning {
            id: warning_id("cycle", &cycle),
            severity: if cycle.len() >= 10 {
                Severity::Error
            } else {
                Severity::Warn
            },
            rule: "cycle".into(),
            message: format!("{} files form a dependency cycle", cycle.len()),
            count: cycle.len(),
            subjects: cycle.clone(),
            metrics: m,
            explanation:
                "A group of files depends on each other in a loop, so none can be understood in isolation."
                    .into(),
            suggestion: format!("Break the loop by moving shared code lower, or invert one edge.{cut_text}"),
            locations: cycle.iter().map(Location::file).collect(),
        });
    }

    // ---- module instability ---------------------------------------------
    for unit in &metrics.modules {
        let Some(i) = unit.instability else { continue };
        if unit.ce >= 5 && i >= 0.8 {
            let mut m = BTreeMap::new();
            m.insert("ce".into(), unit.ce as f64);
            m.insert("instability".into(), i as f64);
            out.push(Warning {
                id: warning_id("high_instability", std::slice::from_ref(&unit.unit)),
                severity: Severity::Warn,
                rule: "high_instability".into(),
                message: format!(
                    "module `{}` is unstable (I={i:.2}, Ce={})",
                    unit.unit, unit.ce
                ),
                subjects: vec![unit.unit.clone()],
                count: 1,
                metrics: m,
                explanation:
                    "The module depends on many others and few depend on it, so changes ripple outward."
                        .into(),
                suggestion:
                    "Move shared abstractions down, or reduce its outgoing dependencies.".into(),
                locations: Vec::new(),
            });
        }
    }

    // ---- low cohesion: calibrated, capped ---------------------------------
    let mut low_cohesion: Vec<(&str, usize, usize, f32)> = Vec::new();
    for unit in &metrics.files {
        let symbols = graph.file(&unit.unit).map(|f| f.symbol_count).unwrap_or(0);
        // 15+ symbols in 4+ disconnected groups with no dominant group.
        if symbols >= 15 && unit.cohesion_components >= 4 && unit.cohesion_largest_ratio <= 0.6 {
            low_cohesion.push((
                unit.unit.as_str(),
                symbols,
                unit.cohesion_components,
                unit.cohesion_largest_ratio,
            ));
        }
    }
    low_cohesion.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal))
    });
    let low_cohesion_total = low_cohesion.len();
    for (file, symbols, components, largest) in low_cohesion.into_iter().take(10) {
        let mut m = BTreeMap::new();
        m.insert("components".into(), components as f64);
        m.insert("symbols".into(), symbols as f64);
        m.insert("largest_ratio".into(), largest as f64);
        out.push(Warning {
            id: warning_id("low_cohesion", &[file.to_string()]),
            severity: Severity::Warn,
            rule: "low_cohesion".into(),
            message: format!(
                "`{file}` has {components} unrelated groups across {symbols} symbols (largest {:.0}%)",
                largest * 100.0
            ),
            subjects: vec![file.to_string()],
            count: 1,
            metrics: m,
            explanation:
                "The file contains several clusters that do not call each other, so it likely does several jobs."
                    .into(),
            suggestion: "Split the file along its groups into separate modules.".into(),
            locations: vec![Location::file(file)],
        });
    }
    if low_cohesion_total > 10 {
        let mut m = BTreeMap::new();
        m.insert("files".into(), low_cohesion_total as f64);
        out.push(Warning {
            id: warning_id("low_cohesion_volume", &[low_cohesion_total.to_string()]),
            severity: Severity::Info,
            rule: "low_cohesion".into(),
            message: format!(
                "{low_cohesion_total} files fall below the cohesion threshold; the top 10 are listed"
            ),
            subjects: Vec::new(),
            count: low_cohesion_total,
            metrics: m,
            explanation:
                "A threshold that matches a large share of the codebase is measuring itself. The listed files are the strongest cases."
                    .into(),
            suggestion:
                "Treat these as candidates, not verdicts; raise the thresholds in config if this codebase is intentionally file-per-feature.".into(),
            locations: Vec::new(),
        });
    }

    // ---- god file: size floor before degree means anything ----------------
    for (path, info) in graph.files() {
        let big_enough = info.loc >= 300 || info.symbol_count >= 40;
        if !big_enough {
            continue;
        }
        let deg = degree.get(path.as_str()).copied().unwrap_or(0);
        if info.symbol_count >= 60 || info.loc >= 1000 || deg >= 150 {
            let mut syms: Vec<&crate::model::Symbol> = symbols_by_file
                .get(path.as_str())
                .cloned()
                .unwrap_or_default();
            syms.sort_by_key(|s| {
                std::cmp::Reverse(
                    s.end_line.unwrap_or(s.line.unwrap_or(0)) as i64 - s.line.unwrap_or(0) as i64,
                )
            });
            let top: Vec<Location> = syms
                .iter()
                .take(3)
                .filter_map(|s| {
                    let line = s.line?;
                    Some(Location::symbol(
                        path.clone(),
                        s.name.clone(),
                        line,
                        s.end_line.unwrap_or(line),
                    ))
                })
                .collect();
            let top_text = if top.is_empty() {
                String::new()
            } else {
                format!(
                    " Largest: {}.",
                    top.iter()
                        .map(Location::display)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let severity = if info.symbol_count >= 120 || info.loc >= 2500 {
                Severity::Error
            } else {
                Severity::Warn
            };
            let mut m = BTreeMap::new();
            m.insert("symbols".into(), info.symbol_count as f64);
            m.insert("loc".into(), info.loc as f64);
            m.insert("degree".into(), deg as f64);
            out.push(Warning {
                id: warning_id("god_file", std::slice::from_ref(path)),
                severity,
                rule: "god_file".into(),
                message: format!(
                    "`{path}` is large: {} symbols, {} LOC, degree {deg}.{top_text}",
                    info.symbol_count, info.loc
                ),
                subjects: vec![path.clone()],
                count: 1,
                metrics: m,
                explanation: "Very large files are hard to review and change safely.".into(),
                suggestion: "Extract cohesive parts into smaller files.".into(),
                locations: top,
            });
        }
    }

    // ---- layer violations -------------------------------------------------
    for (a, b) in cross.keys() {
        let ma = assigner.module_of(a);
        let mb = assigner.module_of(b);
        if ma == mb {
            continue;
        }
        if let Some(allowed) = assigner.allowed(&ma) {
            if !allowed.iter().any(|x| x == &mb) {
                let subjects = vec![format!("{ma} -> {mb}")];
                out.push(Warning {
                    id: warning_id("layer_violation", &subjects),
                    severity: Severity::Error,
                    rule: "layer_violation".into(),
                    message: format!("`{ma}` depends on `{mb}`, which is not allowed"),
                    subjects,
                    count: 1,
                    metrics: BTreeMap::new(),
                    explanation:
                        "A configured architecture rule says this dependency must not exist."
                            .into(),
                    suggestion:
                        "Move the shared code down, or route the call through an allowed module."
                            .into(),
                    locations: vec![Location::file(a.clone())],
                });
            }
        }
    }

    // ---- module fan-out ---------------------------------------------------
    for unit in &metrics.modules {
        if unit.ce > 8 {
            let mut m = BTreeMap::new();
            m.insert("ce".into(), unit.ce as f64);
            out.push(Warning {
                id: warning_id("fan_out_module", std::slice::from_ref(&unit.unit)),
                severity: Severity::Warn,
                rule: "fan_out_module".into(),
                message: format!("module `{}` depends on {} modules", unit.unit, unit.ce),
                subjects: vec![unit.unit.clone()],
                count: 1,
                metrics: m,
                explanation: "A module with many outgoing dependencies is expensive to change."
                    .into(),
                suggestion: "Consider an intermediary or merging tightly coupled modules.".into(),
                locations: Vec::new(),
            });
        }
    }

    // ---- complex function: symbol-level, JSX-aware -----------------------
    for (path, symbols) in &symbols_by_file {
        let worst = symbols
            .iter()
            .filter(|s| s.complexity.is_some())
            .max_by_key(|s| s.complexity.unwrap_or(0));
        let Some(worst) = worst else { continue };
        let cyclomatic = worst.complexity.unwrap_or(0);
        // Render functions are scored on branching outside JSX; they are capped
        // at WARN so ERROR keeps meaning "this will probably break".
        let severity = if worst.is_render {
            if cyclomatic >= 20 {
                Severity::Warn
            } else {
                continue;
            }
        } else if cyclomatic >= 45 {
            Severity::Error
        } else if cyclomatic >= 20 {
            Severity::Warn
        } else {
            continue;
        };
        let line = worst.line.unwrap_or(0);
        let end_line = worst.end_line.unwrap_or(line);
        let mut m = BTreeMap::new();
        m.insert("cyclomatic".into(), cyclomatic as f64);
        out.push(Warning {
            id: warning_id("complex_function", &[format!("{path}::{}", worst.name)]),
            severity,
            rule: "complex_function".into(),
            message: format!(
                "`{path}::{}:{line}–{end_line}` has cyclomatic complexity {cyclomatic}{}",
                worst.name,
                if worst.is_render { " (render function)" } else { "" }
            ),
            subjects: vec![worst.name.clone()],
            count: 1,
            metrics: m,
            explanation:
                "High branching complexity makes a function hard to test and review.".into(),
            suggestion: if worst.is_render {
                "Render functions branch on data. Extract each branch into a component or a lookup table instead of nesting conditionals in the markup."
                    .into()
            } else {
                "Extract branches into named functions or a lookup table.".into()
            },
            locations: vec![Location::symbol(
                path.to_string(),
                worst.name.clone(),
                line,
                end_line,
            )],
        });
    }

    // ---- main sequence ----------------------------------------------------
    for unit in &metrics.modules {
        if let Some(d) = unit.distance {
            let symbols = graph
                .symbols()
                .iter()
                .filter(|s| assigner.module_of(&s.file) == unit.unit)
                .count();
            if d >= 0.7 && symbols >= 5 {
                let mut m = BTreeMap::new();
                m.insert("distance".into(), d as f64);
                out.push(Warning {
                    id: warning_id("main_sequence", std::slice::from_ref(&unit.unit)),
                    severity: Severity::Info,
                    rule: "main_sequence".into(),
                    message: format!(
                        "module `{}` is far from the main sequence (D={d:.2})",
                        unit.unit
                    ),
                    subjects: vec![unit.unit.clone()],
                    count: 1,
                    metrics: m,
                    explanation:
                        "The module is neither stable-and-abstract nor unstable-and-concrete."
                            .into(),
                    suggestion:
                        "Add interfaces if it is stable, or reduce abstraction if it is volatile."
                            .into(),
                    locations: Vec::new(),
                });
            }
        }
    }

    // ---- index quality (not dead code) ------------------------------------
    let mut incoming: HashSet<&crate::model::SymbolId> = HashSet::new();
    for r in graph.relationships() {
        if !r.kind.is_structural() {
            incoming.insert(&r.target_id);
        }
    }
    let total = graph.symbol_count();
    let unresolved = graph
        .symbols()
        .iter()
        .filter(|s| !incoming.contains(&s.id))
        .count();
    if total > 0 && unresolved as f64 / total as f64 > 0.3 {
        let pct = unresolved as f64 / total as f64 * 100.0;
        let mut m = BTreeMap::new();
        m.insert("symbols_without_incoming".into(), unresolved as f64);
        m.insert("total_symbols".into(), total as f64);
        m.insert("percent".into(), pct);
        out.push(Warning {
            id: warning_id("index_quality", &[format!("{unresolved}/{total}")]),
            severity: Severity::Info,
            rule: "index_quality".into(),
            message: format!(
                "{unresolved} of {total} symbols ({pct:.0}%) have no resolved incoming reference"
            ),
            subjects: Vec::new(),
            count: unresolved,
            metrics: m,
            explanation:
                "This is an index-quality number, not a dead-code list: framework conventions, re-exports and dynamic dispatch leave references unresolved, so most of these are used in practice."
                    .into(),
            suggestion:
                "Improve resolution (framework routing, re-exports) before treating any of these as unused."
                    .into(),
            locations: Vec::new(),
        });
    }

    // ---- suppression -------------------------------------------------------
    let matchers: Vec<(String, GlobMatcher)> = config
        .suppress
        .iter()
        .filter_map(|rule| {
            Glob::new(&rule.path)
                .ok()
                .map(|g| (rule.rule.clone(), g.compile_matcher()))
        })
        .collect();
    let mut suppressed = Suppressed::default();
    let before = out.len();
    out.retain(|w| {
        if is_suppressed(w, graph, &matchers) {
            suppressed.config += 1;
            false
        } else {
            true
        }
    });
    let after_config = out.len();
    out.retain(|w| {
        if is_inline_suppressed(w, graph) {
            suppressed.inline += 1;
            false
        } else {
            true
        }
    });
    let _ = (before, after_config);
    if suppressed.config + suppressed.inline > 0 {
        let mut m = BTreeMap::new();
        m.insert("config".into(), suppressed.config as f64);
        m.insert("inline".into(), suppressed.inline as f64);
        out.push(Warning {
            id: warning_id("suppressed", &[
                suppressed.config.to_string(),
                suppressed.inline.to_string(),
            ]),
            severity: Severity::Info,
            rule: "suppressed".into(),
            message: format!(
                "{} finding(s) suppressed ({} by config, {} inline)",
                suppressed.config + suppressed.inline,
                suppressed.config,
                suppressed.inline
            ),
            subjects: Vec::new(),
            count: suppressed.config + suppressed.inline,
            metrics: m,
            explanation:
                "Accepted debt is recorded next to the code or in the committed config, so the decision is reviewable."
                    .into(),
            suggestion: "Run with an empty `suppress` config to see everything again.".into(),
            locations: Vec::new(),
        });
    }

    out.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.rule.cmp(&b.rule))
            .then_with(|| a.message.cmp(&b.message))
    });
    (out, suppressed)
}

/// Config suppression: rule + glob over the finding's files.
fn is_suppressed(w: &Warning, _graph: &CodeGraph, matchers: &[(String, GlobMatcher)]) -> bool {
    w.locations.iter().any(|loc| {
        matchers
            .iter()
            .any(|(rule, matcher)| rule == &w.rule && matcher.is_match(&loc.file))
    })
}

/// Inline suppression: `codegraph-ignore <rule>` within a few lines of the
/// finding, or a bare `codegraph-ignore` for the whole file.
fn is_inline_suppressed(w: &Warning, graph: &CodeGraph) -> bool {
    w.locations.iter().any(|loc| {
        let Some(info) = graph.file(&loc.file) else {
            return false;
        };
        info.ignores.iter().any(|ig| {
            let matches_rule = ig.rules.is_empty() || ig.rules.iter().any(|r| r == &w.rule);
            if !matches_rule {
                return false;
            }
            if ig.rules.is_empty() {
                return true;
            }
            match loc.line {
                Some(line) => {
                    let start = line.saturating_sub(2);
                    let end = line.saturating_add(12);
                    ig.line >= start && ig.line <= end
                }
                None => true,
            }
        })
    })
}

/// File → file cross edges (non-structural, above min confidence).
pub fn cross_edges(graph: &CodeGraph, config: &ProjectConfig) -> HashMap<(String, String), usize> {
    let file_of: HashMap<&crate::model::SymbolId, &str> = graph
        .symbols()
        .iter()
        .map(|s| (&s.id, s.file.as_str()))
        .collect();
    let mut cross: HashMap<(String, String), usize> = HashMap::new();
    for r in graph.relationships() {
        if r.kind.is_structural() || r.confidence < config.min_confidence {
            continue;
        }
        if config.weight_for(r.kind.short()) <= 0.0 {
            continue;
        }
        let (Some(&sf), Some(&tf)) = (file_of.get(&r.source_id), file_of.get(&r.target_id)) else {
            continue;
        };
        if sf != tf {
            *cross.entry((sf.to_string(), tf.to_string())).or_default() += 1;
        }
    }
    cross
}

/// Whether a symbol is likely a React render function (used by tests).
pub fn looks_like_render(sym: &crate::model::Symbol) -> bool {
    sym.is_render
        && sym
            .name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
        && !matches!(sym.kind, SymbolKind::Class | SymbolKind::Interface)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ModuleDefConfig, SuppressRule};
    use crate::metrics::compute;
    use crate::model::{FileInfo, FileStatus, Relationship, RelationshipKind, Symbol, SymbolId};

    fn sym(id: &str, file: &str) -> Symbol {
        Symbol::new(SymbolId(id.into()), id, SymbolKind::Function, file)
    }

    fn rel(id: u64, from: &str, to: &str) -> Relationship {
        Relationship::indexed(
            id,
            SymbolId(from.into()),
            SymbolId(to.into()),
            RelationshipKind::Imports,
            None,
            1.0,
            "import",
        )
    }

    fn fi(path: &str) -> (String, FileInfo) {
        (
            path.into(),
            FileInfo {
                path: path.into(),
                lang: None,
                loc: 10,
                symbol_count: 1,
                status: FileStatus::Indexed,
                complexity_sum: 0,
                complexity_max: 0,
                function_count: 0,
                ignores: Vec::new(),
            },
        )
    }

    fn cycle_graph() -> CodeGraph {
        CodeGraph::new_with_files(
            vec![sym("a", "a.ts"), sym("b", "b.ts")],
            vec![rel(1, "a", "b"), rel(2, "b", "a")],
            [fi("a.ts"), fi("b.ts")].into_iter().collect(),
        )
        .unwrap()
    }

    #[test]
    fn detects_file_cycle_with_exact_cuts() {
        let graph = cycle_graph();
        let assigner = ModuleAssigner::new(&[]);
        let metrics = compute(&graph, &assigner, &ProjectConfig::default());
        let warnings = run_warnings(&graph, &assigner, &metrics, &ProjectConfig::default());
        let cycle = warnings
            .iter()
            .find(|w| w.rule == "cycle")
            .expect("cycle warning");
        assert!(
            cycle.suggestion.contains("a.ts -> b.ts") || cycle.suggestion.contains("b.ts -> a.ts"),
            "suggestion should list the edges to cut: {}",
            cycle.suggestion
        );
        assert_eq!(cycle.locations.len(), 2);
        assert_eq!(cycle.locations[0].display(), "a.ts");
    }

    #[test]
    fn detects_layer_violation() {
        let graph = CodeGraph::new_with_files(
            vec![sym("a", "a.ts"), sym("b", "b.ts")],
            vec![rel(1, "a", "b"), rel(2, "b", "a")],
            [fi("a.ts"), fi("b.ts")].into_iter().collect(),
        )
        .unwrap();
        let assigner = ModuleAssigner::new(&[
            ModuleDefConfig {
                name: "A".into(),
                paths: vec!["a.ts".into()],
                allowed_deps: Some(vec!["B".into()]),
            },
            ModuleDefConfig {
                name: "B".into(),
                paths: vec!["b.ts".into()],
                allowed_deps: Some(vec![]),
            },
        ]);
        let metrics = compute(&graph, &assigner, &ProjectConfig::default());
        let warnings = run_warnings(&graph, &assigner, &metrics, &ProjectConfig::default());
        assert!(warnings.iter().any(|w| w.rule == "layer_violation"));
    }

    #[test]
    fn ids_are_stable_and_subject_sensitive() {
        let a = warning_id("cycle", &["b".into(), "a".into()]);
        let b = warning_id("cycle", &["a".into(), "b".into()]);
        let c = warning_id("cycle", &["a".into()]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    /// A 22-line shared type with 572 inbound references must NOT be a god file.
    #[test]
    fn small_high_degree_file_is_not_a_god_file() {
        let mut files: HashMap<String, FileInfo> = HashMap::new();
        let mut info = fi("lib/result.ts").1;
        info.loc = 22;
        info.symbol_count = 4;
        files.insert("lib/result.ts".into(), info);
        let mut symbols = vec![sym("r", "lib/result.ts")];
        let mut rels = Vec::new();
        for i in 0..50 {
            let id = format!("u{i}");
            symbols.push(sym(&id, "src/user.ts"));
            rels.push(rel(i + 1, &id, "r"));
        }
        let graph = CodeGraph::new_with_files(symbols, rels, files).unwrap();
        let assigner = ModuleAssigner::new(&[]);
        let metrics = compute(&graph, &assigner, &ProjectConfig::default());
        let warnings = run_warnings(&graph, &assigner, &metrics, &ProjectConfig::default());
        assert!(
            !warnings.iter().any(|w| w.rule == "god_file"),
            "high fan-in alone must not trigger god_file"
        );
    }

    #[test]
    fn god_file_names_its_largest_symbols() {
        let mut files: HashMap<String, FileInfo> = HashMap::new();
        let mut info = fi("src/huge.ts").1;
        info.loc = 1200;
        info.symbol_count = 70;
        files.insert("src/huge.ts".into(), info);
        let mut big = sym("big", "src/huge.ts");
        big.line = Some(10);
        big.end_line = Some(400);
        let mut medium = sym("medium", "src/huge.ts");
        medium.line = Some(410);
        medium.end_line = Some(600);
        let graph = CodeGraph::new_with_files(vec![big, medium], Vec::new(), files).unwrap();
        let assigner = ModuleAssigner::new(&[]);
        let metrics = compute(&graph, &assigner, &ProjectConfig::default());
        let warnings = run_warnings(&graph, &assigner, &metrics, &ProjectConfig::default());
        let god = warnings
            .iter()
            .find(|w| w.rule == "god_file")
            .expect("god file");
        assert!(
            god.message.contains("src/huge.ts::big:10–400"),
            "{}",
            god.message
        );
        assert!(god.message.contains("medium"), "{}", god.message);
    }

    #[test]
    fn complex_function_points_at_the_symbol_and_caps_render_functions() {
        let mut files: HashMap<String, FileInfo> = HashMap::new();
        files.insert("app/page.tsx".into(), fi("app/page.tsx").1);
        let mut page = Symbol::new(
            SymbolId("p".into()),
            "Page",
            SymbolKind::Function,
            "app/page.tsx",
        );
        page.line = Some(12);
        page.end_line = Some(120);
        page.complexity = Some(60);
        page.is_render = true;
        let graph = CodeGraph::new_with_files(vec![page], Vec::new(), files).unwrap();
        let assigner = ModuleAssigner::new(&[]);
        let metrics = compute(&graph, &assigner, &ProjectConfig::default());
        let warnings = run_warnings(&graph, &assigner, &metrics, &ProjectConfig::default());
        let cx = warnings
            .iter()
            .find(|w| w.rule == "complex_function")
            .expect("complexity warning");
        assert_eq!(cx.severity, Severity::Warn, "render functions never ERROR");
        assert!(
            cx.message.contains("app/page.tsx::Page:12–120"),
            "{}",
            cx.message
        );
        assert_eq!(cx.locations[0].symbol.as_deref(), Some("Page"));
    }

    #[test]
    fn inline_ignore_and_config_suppress_findings() {
        let mut files: HashMap<String, FileInfo> = HashMap::new();
        let mut info = fi("src/a.ts").1;
        info.ignores = vec![crate::indexer::facts::Ignore {
            line: 3,
            rules: vec!["complex_function".into()],
        }];
        files.insert("src/a.ts".into(), info);
        let mut f = Symbol::new(
            SymbolId("f".into()),
            "big",
            SymbolKind::Function,
            "src/a.ts",
        );
        f.line = Some(3);
        f.end_line = Some(90);
        f.complexity = Some(50);
        let graph = CodeGraph::new_with_files(vec![f], Vec::new(), files).unwrap();
        let assigner = ModuleAssigner::new(&[]);
        let metrics = compute(&graph, &assigner, &ProjectConfig::default());
        let (warnings, suppressed) =
            run_warnings_reporting(&graph, &assigner, &metrics, &ProjectConfig::default());
        assert!(!warnings.iter().any(|w| w.rule == "complex_function"));
        assert_eq!(suppressed.inline, 1);
        assert!(warnings.iter().any(|w| w.rule == "suppressed"));

        // Same finding, suppressed by committed config instead.
        let config = ProjectConfig {
            suppress: vec![SuppressRule {
                rule: "complex_function".into(),
                path: "src/*.ts".into(),
                reason: Some("generated".into()),
            }],
            ..Default::default()
        };
        let mut f = Symbol::new(
            SymbolId("g".into()),
            "big2",
            SymbolKind::Function,
            "src/b.ts",
        );
        f.line = Some(1);
        f.end_line = Some(80);
        f.complexity = Some(50);
        let graph2 =
            CodeGraph::new_with_files(vec![f], Vec::new(), [fi("src/b.ts")].into_iter().collect())
                .unwrap();
        let metrics = compute(&graph2, &assigner, &config);
        let (warnings, suppressed) = run_warnings_reporting(&graph2, &assigner, &metrics, &config);
        assert!(!warnings.iter().any(|w| w.rule == "complex_function"));
        assert_eq!(suppressed.config, 1);
    }

    #[test]
    fn index_quality_is_not_phrased_as_dead_code() {
        let mut symbols = Vec::new();
        for i in 0..10 {
            symbols.push(Symbol::new(
                SymbolId(format!("s{i}")),
                format!("s{i}"),
                SymbolKind::Function,
                "src/a.ts",
            ));
        }
        let graph =
            CodeGraph::new_with_files(symbols, Vec::new(), [fi("src/a.ts")].into_iter().collect())
                .unwrap();
        let assigner = ModuleAssigner::new(&[]);
        let metrics = compute(&graph, &assigner, &ProjectConfig::default());
        let warnings = run_warnings(&graph, &assigner, &metrics, &ProjectConfig::default());
        let q = warnings
            .iter()
            .find(|w| w.rule == "index_quality")
            .expect("index quality");
        assert_eq!(q.severity, Severity::Info);
        assert!(q.explanation.contains("not a dead-code list"));
        assert!(!warnings.iter().any(|w| w.rule == "orphan"));
    }
}
