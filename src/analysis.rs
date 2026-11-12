//! Static-analysis helpers over a loaded [`CodeGraph`]:
//! circular-dependency detection and coupling/cohesion metrics.
//!
//! These implement the checks from the `typescript-circular-dependency` and
//! `cohesion-coupling` skills directly against the graph data:
//! - **Cycles**: strongly-connected components with 2+ symbols are dependency
//!   cycles (e.g. `a.ts -> b.ts -> a.ts`).
//! - **Coupling**: per-file afferent (Ca, incoming) and efferent (Ce,
//!   outgoing) coupling, instability = Ce / (Ca + Ce), and a cohesion proxy
//!   (within-file edges / all edges touching the file).

use std::collections::{HashMap, HashSet};

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};

use crate::graph::CodeGraph;
use crate::model::SymbolId;

/// A dependency cycle: a strongly-connected component of 2+ symbols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    pub nodes: Vec<SymbolId>,
}

/// Coupling/cohesion metrics for one file (or module).
#[derive(Debug, Clone)]
pub struct FileMetrics {
    pub file: String,
    /// Afferent coupling: relationships coming into this file from others.
    pub ca: usize,
    /// Efferent coupling: relationships going out of this file to others.
    pub ce: usize,
    /// Internal relationships (both ends in this file).
    pub internal: usize,
    /// Instability = ce / (ca + ce); 0 = stable, 1 = unstable.
    pub instability: f32,
    /// Cohesion proxy = internal / (internal + ce); 1 = fully cohesive.
    pub cohesion: f32,
}

/// Find all dependency cycles (SCCs with more than one symbol).
///
/// Uses Tarjan's algorithm, so it is O(V + E) and scales to the full graph.
pub fn find_cycles(graph: &CodeGraph) -> Vec<Cycle> {
    let symbols = graph.symbols();
    let mut index: HashMap<&SymbolId, usize> = HashMap::with_capacity(symbols.len());
    for (i, s) in symbols.iter().enumerate() {
        index.insert(&s.id, i);
    }

    let mut g: DiGraph<usize, ()> = DiGraph::with_capacity(symbols.len(), 0);
    for i in 0..symbols.len() {
        g.add_node(i);
    }
    for r in graph.relationships() {
        if let (Some(&a), Some(&b)) = (index.get(&r.source_id), index.get(&r.target_id)) {
            if a != b {
                g.add_edge(NodeIndex::new(a), NodeIndex::new(b), ());
            }
        }
    }

    tarjan_scc(&g)
        .into_iter()
        .filter(|component| component.len() > 1)
        .map(|component| {
            let mut nodes: Vec<SymbolId> = component
                .into_iter()
                .map(|n| symbols[g[n]].id.clone())
                .collect();
            nodes.sort();
            Cycle { nodes }
        })
        .collect()
}

/// Compute coupling/cohesion metrics for every file in the graph.
///
/// Result is sorted by instability (most unstable first), which is where the
/// `cohesion-coupling` skill says to focus refactoring effort.
pub fn coupling_report(graph: &CodeGraph) -> Vec<FileMetrics> {
    let mut file_of: HashMap<&SymbolId, &str> = HashMap::new();
    let mut files: HashSet<&str> = HashSet::new();
    for s in graph.symbols() {
        file_of.insert(&s.id, &s.file);
        files.insert(&s.file);
    }

    let mut ca: HashMap<&str, usize> = HashMap::new();
    let mut ce: HashMap<&str, usize> = HashMap::new();
    let mut internal: HashMap<&str, usize> = HashMap::new();

    for r in graph.relationships() {
        let (Some(&sf), Some(&tf)) = (file_of.get(&r.source_id), file_of.get(&r.target_id)) else {
            continue;
        };
        if sf == tf {
            *internal.entry(sf).or_default() += 1;
        } else {
            *ce.entry(sf).or_default() += 1;
            *ca.entry(tf).or_default() += 1;
        }
    }

    let mut metrics: Vec<FileMetrics> = files
        .into_iter()
        .map(|file| {
            let ca = *ca.get(file).unwrap_or(&0);
            let ce = *ce.get(file).unwrap_or(&0);
            let internal = *internal.get(file).unwrap_or(&0);
            let total = ca + ce;
            let instability = if total == 0 {
                0.0
            } else {
                ce as f32 / total as f32
            };
            let cohesion_total = internal + ce;
            let cohesion = if cohesion_total == 0 {
                1.0
            } else {
                internal as f32 / cohesion_total as f32
            };
            FileMetrics {
                file: file.to_string(),
                ca,
                ce,
                internal,
                instability,
                cohesion,
            }
        })
        .collect();

    metrics.sort_by(|a, b| {
        b.instability
            .partial_cmp(&a.instability)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    metrics
}

/// A one-line summary of the audit for the UI.
pub fn summarize(cycles: &[Cycle], report: &[FileMetrics]) -> String {
    let unstable = report
        .iter()
        .filter(|m| m.instability >= 0.8 && m.ce >= 3)
        .count();
    format!(
        "{} cycles · {} files · {} unstable (high Ce)",
        cycles.len(),
        report.len(),
        unstable
    )
}

/// Find file-level dependency cycles (SCCs of the file graph).
pub fn find_file_cycles(graph: &CodeGraph, min_confidence: f32) -> Vec<Vec<String>> {
    use std::collections::HashMap;
    let files: Vec<&str> = graph.files().keys().map(|s| s.as_str()).collect();
    if files.is_empty() {
        return Vec::new();
    }
    let index: HashMap<&str, usize> = files.iter().enumerate().map(|(i, f)| (*f, i)).collect();
    let mut g: petgraph::graph::DiGraph<usize, ()> = petgraph::graph::DiGraph::new();
    for i in 0..files.len() {
        g.add_node(i);
    }
    let file_of: HashMap<&SymbolId, &str> = graph
        .symbols()
        .iter()
        .map(|s| (&s.id, s.file.as_str()))
        .collect();
    let mut seen = std::collections::HashSet::new();
    for r in graph.relationships() {
        if r.kind.is_structural() || r.confidence < min_confidence {
            continue;
        }
        let (Some(&sf), Some(&tf)) = (file_of.get(&r.source_id), file_of.get(&r.target_id)) else {
            continue;
        };
        if sf == tf {
            continue;
        }
        if let (Some(&a), Some(&b)) = (index.get(sf), index.get(tf)) {
            if seen.insert((a, b)) && a != b {
                g.add_edge(
                    petgraph::graph::NodeIndex::new(a),
                    petgraph::graph::NodeIndex::new(b),
                    (),
                );
            }
        }
    }
    petgraph::algo::tarjan_scc(&g)
        .into_iter()
        .filter(|c| c.len() > 1)
        .map(|c| {
            let mut names: Vec<String> = c.into_iter().map(|n| files[g[n]].to_string()).collect();
            names.sort();
            names
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Relationship, RelationshipKind, Symbol, SymbolKind};

    fn sym(id: &str, name: &str, file: &str) -> Symbol {
        Symbol::new(SymbolId(id.into()), name, SymbolKind::Function, file)
    }

    fn rel(id: u64, from: &str, to: &str) -> Relationship {
        Relationship::new(
            id,
            SymbolId(from.into()),
            SymbolId(to.into()),
            RelationshipKind::Imports,
        )
    }

    #[test]
    fn detects_simple_cycle() {
        // a -> b -> a is a cycle; c is not.
        let graph = CodeGraph::new(
            vec![
                sym("a", "a", "a.ts"),
                sym("b", "b", "b.ts"),
                sym("c", "c", "c.ts"),
            ],
            vec![rel(1, "a", "b"), rel(2, "b", "a"), rel(3, "b", "c")],
        )
        .expect("valid graph");

        let cycles = find_cycles(&graph);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].nodes.len(), 2);
        assert!(cycles[0].nodes.contains(&SymbolId("a".into())));
        assert!(cycles[0].nodes.contains(&SymbolId("b".into())));
    }

    #[test]
    fn no_cycles_in_dag() {
        let graph = CodeGraph::new(
            vec![sym("a", "a", "a.ts"), sym("b", "b", "b.ts")],
            vec![rel(1, "a", "b")],
        )
        .expect("valid graph");
        assert!(find_cycles(&graph).is_empty());
    }

    #[test]
    fn coupling_counts_incoming_and_outgoing() {
        // a.ts -> b.ts twice, b.ts -> c.ts once, plus one internal in b.
        let graph = CodeGraph::new(
            vec![
                sym("a", "a", "a.ts"),
                sym("b", "b", "b.ts"),
                sym("b2", "b2", "b.ts"),
                sym("c", "c", "c.ts"),
            ],
            vec![
                rel(1, "a", "b"),
                rel(2, "a", "b2"),
                rel(3, "b", "c"),
                rel(4, "b", "b2"),
            ],
        )
        .expect("valid graph");

        let report = coupling_report(&graph);
        let b = report.iter().find(|m| m.file == "b.ts").expect("b.ts");
        assert_eq!(b.ca, 2); // from a.ts (x2)
        assert_eq!(b.ce, 1); // to c.ts
        assert_eq!(b.internal, 1); // b -> b2
                                   // instability = ce / (ca + ce) = 1/3
        assert!((b.instability - 1.0 / 3.0).abs() < 1e-4);
    }
}
