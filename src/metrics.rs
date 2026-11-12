//! Engineering metrics: coupling, cohesion, abstractness, complexity.
//!
//! These are heuristics (not standards-mandated formulas). Coupling/cohesion
//! are conceptually based on the GFG "Coupling vs Cohesion" definitions:
//! coupling is dependency between units, cohesion is how strongly the elements
//! inside a unit belong together. See `docs/METRICS.md`.

use std::collections::{HashMap, HashSet};

use crate::config::ProjectConfig;
use crate::graph::CodeGraph;
use crate::model::{EdgeDirection, SymbolId};
use crate::modules::ModuleAssigner;

#[derive(Debug, Clone, Default)]
pub struct UnitMetrics {
    pub unit: String,
    pub is_module: bool,
    pub ca: usize,
    pub ce: usize,
    pub ca_edges: usize,
    pub ce_edges: usize,
    pub cbo: usize,
    pub instability: Option<f32>,
    pub abstractness: Option<f32>,
    pub distance: Option<f32>,
    pub cohesion_components: usize,
    pub cohesion_largest_ratio: f32,
    pub internal_ratio: f32,
    pub internal_edge_ratio: f32,
    pub complexity_sum: u32,
    pub complexity_max: u32,
    pub function_count: usize,
    pub loc: usize,
    /// Number of files in the unit (1 for a file, N for a module).
    pub file_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct MetricsBundle {
    pub files: Vec<UnitMetrics>,
    pub modules: Vec<UnitMetrics>,
}

/// Compute file- and module-level metrics.
pub fn compute(
    graph: &CodeGraph,
    assigner: &ModuleAssigner,
    config: &ProjectConfig,
) -> MetricsBundle {
    let file_of: HashMap<&SymbolId, &str> = graph
        .symbols()
        .iter()
        .map(|s| (&s.id, s.file.as_str()))
        .collect();

    let mut cross: HashMap<(&str, &str), usize> = HashMap::new();
    let mut internal_file: HashMap<&str, usize> = HashMap::new();
    for rel in graph.relationships() {
        if rel.kind.is_structural() || rel.confidence < config.min_confidence {
            continue;
        }
        let weight = config.weight_for(rel.kind.short());
        if weight <= 0.0 {
            continue;
        }
        let (Some(&sf), Some(&tf)) = (file_of.get(&rel.source_id), file_of.get(&rel.target_id))
        else {
            continue;
        };
        if sf == tf {
            *internal_file.entry(sf).or_default() += 1;
        } else {
            *cross.entry((sf, tf)).or_default() += 1;
        }
    }

    let files = build_file_metrics(graph, &file_of, &cross, &internal_file, config);
    let modules = build_module_metrics(graph, assigner, &file_of, &cross, &internal_file, config);
    MetricsBundle { files, modules }
}

fn build_file_metrics(
    graph: &CodeGraph,
    file_of: &HashMap<&SymbolId, &str>,
    cross: &HashMap<(&str, &str), usize>,
    internal_file: &HashMap<&str, usize>,
    config: &ProjectConfig,
) -> Vec<UnitMetrics> {
    let mut ce: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut ca: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut ce_edges: HashMap<&str, usize> = HashMap::new();
    let mut ca_edges: HashMap<&str, usize> = HashMap::new();
    for ((s, t), n) in cross {
        ce.entry(s).or_default().insert(t);
        ca.entry(t).or_default().insert(s);
        *ce_edges.entry(s).or_default() += n;
        *ca_edges.entry(t).or_default() += n;
    }

    let mut symbols_by_file: HashMap<&str, Vec<&SymbolId>> = HashMap::new();
    for s in graph.symbols() {
        symbols_by_file
            .entry(s.file.as_str())
            .or_default()
            .push(&s.id);
    }

    let mut out = Vec::new();
    for (file, info) in graph.files() {
        let ce_set = ce.get(file.as_str()).cloned().unwrap_or_default();
        let ca_set = ca.get(file.as_str()).cloned().unwrap_or_default();
        let ce_n = ce_set.len();
        let ca_n = ca_set.len();
        let instability = if ca_n + ce_n == 0 {
            None
        } else {
            Some(ce_n as f32 / (ca_n + ce_n) as f32)
        };

        let internal = *internal_file.get(file.as_str()).unwrap_or(&0);
        let outgoing = *ce_edges.get(file.as_str()).unwrap_or(&0);
        let internal_ratio = ratio(internal, internal + outgoing);
        let internal_edge_ratio = ratio(internal, internal + outgoing);

        let (components, largest) = file_cohesion(graph, file_of, file, config);
        let mut cbo_set = ce_set;
        cbo_set.extend(ca_set.iter().copied());

        let _ = info;
        out.push(UnitMetrics {
            unit: file.clone(),
            is_module: false,
            ca: ca_n,
            ce: ce_n,
            ca_edges: *ca_edges.get(file.as_str()).unwrap_or(&0),
            ce_edges: outgoing,
            cbo: cbo_set.len(),
            instability,
            abstractness: None,
            distance: None,
            cohesion_components: components,
            cohesion_largest_ratio: largest,
            internal_ratio,
            internal_edge_ratio,
            complexity_sum: info.complexity_sum,
            complexity_max: info.complexity_max,
            function_count: info.function_count,
            loc: info.loc,
            file_count: 1,
        });
    }
    out.sort_by(|a, b| a.unit.cmp(&b.unit));
    out
}

#[allow(clippy::too_many_arguments)]
fn build_module_metrics(
    graph: &CodeGraph,
    assigner: &ModuleAssigner,
    file_of: &HashMap<&SymbolId, &str>,
    cross: &HashMap<(&str, &str), usize>,
    internal_file: &HashMap<&str, usize>,
    config: &ProjectConfig,
) -> Vec<UnitMetrics> {
    let _ = config;
    let mut files_by_module: HashMap<String, Vec<&str>> = HashMap::new();
    for file in graph.files().keys() {
        files_by_module
            .entry(assigner.module_of(file))
            .or_default()
            .push(file.as_str());
    }

    let mut mod_pair: HashMap<(String, String), usize> = HashMap::new();
    let mut mod_internal: HashMap<String, usize> = HashMap::new();
    for ((s, t), n) in cross {
        let sm = assigner.module_of(s);
        let tm = assigner.module_of(t);
        if sm == tm {
            *mod_internal.entry(sm).or_default() += n;
        } else {
            *mod_pair.entry((sm, tm)).or_default() += n;
        }
    }
    for (file, n) in internal_file {
        *mod_internal.entry(assigner.module_of(file)).or_default() += n;
    }

    let mut ce_set: HashMap<String, HashSet<String>> = HashMap::new();
    let mut ca_set: HashMap<String, HashSet<String>> = HashMap::new();
    let mut ce_edges: HashMap<String, usize> = HashMap::new();
    let mut ca_edges: HashMap<String, usize> = HashMap::new();
    for ((s, t), n) in &mod_pair {
        ce_set.entry(s.clone()).or_default().insert(t.clone());
        ca_set.entry(t.clone()).or_default().insert(s.clone());
        *ce_edges.entry(s.clone()).or_default() += n;
        *ca_edges.entry(t.clone()).or_default() += n;
    }

    let mut out = Vec::new();
    for (module, files) in files_by_module {
        let ce_n = ce_set.get(&module).map(|s| s.len()).unwrap_or(0);
        let ca_n = ca_set.get(&module).map(|s| s.len()).unwrap_or(0);
        let instability = if ca_n + ce_n == 0 {
            None
        } else {
            Some(ce_n as f32 / (ca_n + ce_n) as f32)
        };

        // Abstractness: type-like vs abstract symbols in the module.
        let mut type_like = 0usize;
        let mut abstract_types = 0usize;
        for s in graph.symbols() {
            if assigner.module_of(&s.file) != module {
                continue;
            }
            if s.kind.is_type_like() {
                type_like += 1;
                if s.kind.is_abstract_type() {
                    abstract_types += 1;
                }
            }
        }
        let abstractness = if type_like == 0 {
            None
        } else {
            Some(abstract_types as f32 / type_like as f32)
        };
        let distance = match (abstractness, instability) {
            (Some(a), Some(i)) => Some((a + i - 1.0).abs()),
            _ => None,
        };

        let internal = *mod_internal.get(&module).unwrap_or(&0);
        let outgoing = *ce_edges.get(&module).unwrap_or(&0);
        let internal_ratio = ratio(internal, internal + outgoing);

        let (components, largest) = module_cohesion(files.as_slice(), cross, cross);

        let mut cbo_set = ce_set.get(&module).cloned().unwrap_or_default();
        cbo_set.extend(ca_set.get(&module).cloned().unwrap_or_default());

        let complexity_sum = files
            .iter()
            .filter_map(|f| graph.file(f))
            .map(|fi| fi.complexity_sum)
            .sum();
        let complexity_max = files
            .iter()
            .filter_map(|f| graph.file(f))
            .map(|fi| fi.complexity_max)
            .max()
            .unwrap_or(0);
        let loc = files
            .iter()
            .filter_map(|f| graph.file(f))
            .map(|fi| fi.loc)
            .sum();

        out.push(UnitMetrics {
            unit: module.clone(),
            is_module: true,
            ca: ca_n,
            ce: ce_n,
            ca_edges: *ca_edges.get(&module).unwrap_or(&0),
            ce_edges: outgoing,
            cbo: cbo_set.len(),
            instability,
            abstractness,
            distance,
            cohesion_components: components,
            cohesion_largest_ratio: largest,
            internal_ratio,
            internal_edge_ratio: internal_ratio,
            complexity_sum,
            complexity_max,
            function_count: 0,
            loc,
            file_count: files.len(),
        });
    }
    out.sort_by(|a, b| a.unit.cmp(&b.unit));
    let _ = file_of;
    out
}

fn file_cohesion(
    graph: &CodeGraph,
    _file_of: &HashMap<&SymbolId, &str>,
    file: &str,
    config: &ProjectConfig,
) -> (usize, f32) {
    let symbols: Vec<&SymbolId> = graph
        .symbols()
        .iter()
        .filter(|s| s.file == file)
        .map(|s| &s.id)
        .collect();
    let index: HashMap<&SymbolId, usize> =
        symbols.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let mut edges = Vec::new();
    for s in &symbols {
        for rel in graph.relationships_for((*s).clone(), EdgeDirection::Outgoing) {
            if rel.kind.is_structural() || rel.confidence < config.min_confidence {
                continue;
            }
            if let (Some(&a), Some(&b)) = (index.get(&rel.source_id), index.get(&rel.target_id)) {
                edges.push((a, b));
            }
        }
    }
    components(symbols.len(), &edges)
}

fn module_cohesion(
    files: &[&str],
    cross: &HashMap<(&str, &str), usize>,
    _internal: &HashMap<(&str, &str), usize>,
) -> (usize, f32) {
    let index: HashMap<&str, usize> = files.iter().enumerate().map(|(i, f)| (*f, i)).collect();
    let mut edges = Vec::new();
    for (s, t) in cross.keys() {
        if let (Some(&a), Some(&b)) = (index.get(s), index.get(t)) {
            edges.push((a, b));
        }
    }
    components(files.len(), &edges)
}

/// Weakly connected components count and largest component ratio.
fn components(n: usize, edges: &[(usize, usize)]) -> (usize, f32) {
    if n == 0 {
        return (0, 1.0);
    }
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for &(a, b) in edges {
        if a < n && b < n {
            let ra = find(&mut parent, a);
            let rb = find(&mut parent, b);
            if ra != rb {
                parent[ra] = rb;
            }
        }
    }
    let mut sizes: HashMap<usize, usize> = HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        *sizes.entry(r).or_default() += 1;
    }
    let count = sizes.len();
    let largest = sizes.values().copied().max().unwrap_or(0);
    (count, largest as f32 / n as f32)
}

fn ratio(a: usize, total: usize) -> f32 {
    if total == 0 {
        1.0
    } else {
        a as f32 / total as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModuleDefConfig;
    use crate::model::{FileInfo, FileStatus, Relationship, RelationshipKind, Symbol, SymbolKind};

    fn sym(id: &str, file: &str, kind: SymbolKind) -> Symbol {
        Symbol::new(SymbolId(id.into()), id, kind, file)
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

    fn file(path: &str, complexity: u32, functions: usize) -> (String, FileInfo) {
        (
            path.to_string(),
            FileInfo {
                path: path.into(),
                lang: Some("typescript".into()),
                loc: 10,
                symbol_count: 1,
                status: FileStatus::Indexed,
                complexity_sum: complexity,
                complexity_max: complexity,
                function_count: functions,
                ignores: Vec::new(),
            },
        )
    }

    fn graph() -> CodeGraph {
        let files = [file("a.ts", 5, 2), file("b.ts", 9, 3), file("c.ts", 1, 1)]
            .into_iter()
            .collect();
        CodeGraph::new_with_files(
            vec![
                sym("a1", "a.ts", SymbolKind::Function),
                sym("b1", "b.ts", SymbolKind::Function),
                sym("c1", "c.ts", SymbolKind::Function),
            ],
            vec![rel(1, "a1", "b1"), rel(2, "b1", "c1"), rel(3, "a1", "b1")],
            files,
        )
        .unwrap()
    }

    #[test]
    fn coupling_counts() {
        let g = graph();
        let assigner = ModuleAssigner::new(&[]);
        let bundle = compute(&g, &assigner, &ProjectConfig::default());
        let a = bundle.files.iter().find(|m| m.unit == "a.ts").unwrap();
        assert_eq!(a.ce, 1, "a depends on b");
        assert_eq!(a.ce_edges, 2);
        assert_eq!(a.ca, 0);
        assert_eq!(a.cbo, 1);

        let b = bundle.files.iter().find(|m| m.unit == "b.ts").unwrap();
        assert_eq!(b.ca, 1);
        assert_eq!(b.ce, 1);
        assert_eq!(b.instability, Some(0.5));

        let c = bundle.files.iter().find(|m| m.unit == "c.ts").unwrap();
        assert_eq!(c.ca, 1);
        assert_eq!(c.ce, 0);
        assert_eq!(c.instability, Some(0.0));
    }

    #[test]
    fn complexity_rollup_from_file_info() {
        let g = graph();
        let bundle = compute(&g, &ModuleAssigner::new(&[]), &ProjectConfig::default());
        let b = bundle.files.iter().find(|m| m.unit == "b.ts").unwrap();
        assert_eq!(b.complexity_sum, 9);
        assert_eq!(b.function_count, 3);
    }

    #[test]
    fn module_metrics_and_layers() {
        let g = graph();
        let assigner = ModuleAssigner::new(&[
            ModuleDefConfig {
                name: "A".into(),
                paths: vec!["a.ts".into()],
                allowed_deps: Some(vec!["B".into()]),
            },
            ModuleDefConfig {
                name: "B".into(),
                paths: vec!["b.ts".into(), "c.ts".into()],
                allowed_deps: None,
            },
        ]);
        let bundle = compute(&g, &assigner, &ProjectConfig::default());
        let a = bundle.modules.iter().find(|m| m.unit == "A").unwrap();
        assert_eq!(a.ce, 1);
        assert_eq!(a.ca, 0);
        let b = bundle.modules.iter().find(|m| m.unit == "B").unwrap();
        assert_eq!(b.ca, 1);
    }

    #[test]
    fn components_merge() {
        assert_eq!(components(3, &[(0, 1), (1, 2)]), (1, 1.0));
        assert_eq!(components(3, &[]), (3, 1.0 / 3.0));
        assert_eq!(components(0, &[]), (0, 1.0));
    }
}
