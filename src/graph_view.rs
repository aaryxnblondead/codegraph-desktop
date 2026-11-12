//! Framework-agnostic graph model, layered layout, and aggregation for the
//! CodeGraph visualizer.
//!
//! This module computes *what* to draw (nodes, edges, positions, sizes) and
//! the whole-graph / neighborhood aggregation. Rendering lives in the UI layer
//! (egui's immediate-mode painter), so the data layer has no GUI dependency.
//!
//! Layout is hierarchical (Sugiyama-style): cycles are condensed into
//! strongly-connected components, then nodes are assigned layers by longest
//! path and ordered within each layer by the barycenter of their parents.

use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};

use crate::model::{Relationship, Symbol, SymbolId};

/// A precomputed graph view ready to be drawn.
#[derive(Debug, Clone)]
pub struct GraphView {
    nodes: Vec<Symbol>,
    /// Edge endpoints as indices into `nodes`.
    edges: Vec<(usize, usize)>,
    /// Node positions in graph space (arbitrary units).
    positions: Vec<(f32, f32)>,
    /// Base node radius per node.
    sizes: Vec<f32>,
    /// Selected node (for highlighting), if any.
    selected: Option<SymbolId>,
}

impl GraphView {
    pub fn new(nodes: Vec<Symbol>, edges: Vec<Relationship>, selected: Option<SymbolId>) -> Self {
        let resolved = resolve_edges(&nodes, &edges);
        let rank = compute_pagerank(nodes.len(), &resolved);
        let sizes = rank.iter().map(|r| 5.0 + r * 2.0).collect();
        Self::assemble(nodes, resolved, sizes, selected)
    }

    pub fn new_with_sizes(
        nodes: Vec<Symbol>,
        edges: Vec<Relationship>,
        selected: Option<SymbolId>,
        sizes: Vec<f32>,
    ) -> Self {
        let resolved = resolve_edges(&nodes, &edges);
        let mut sizes = sizes;
        sizes.resize(nodes.len(), 6.0);
        Self::assemble(nodes, resolved, sizes, selected)
    }

    fn assemble(
        nodes: Vec<Symbol>,
        edges: Vec<(usize, usize)>,
        sizes: Vec<f32>,
        selected: Option<SymbolId>,
    ) -> Self {
        let positions = layered_layout(nodes.len(), &edges);
        Self {
            nodes,
            edges,
            positions,
            sizes,
            selected,
        }
    }

    pub fn nodes(&self) -> &[Symbol] {
        &self.nodes
    }

    pub fn edges(&self) -> &[(usize, usize)] {
        &self.edges
    }

    pub fn positions(&self) -> &[(f32, f32)] {
        &self.positions
    }

    pub fn sizes(&self) -> &[f32] {
        &self.sizes
    }

    pub fn selected(&self) -> Option<&SymbolId> {
        self.selected.as_ref()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Base radius for node `i`.
    pub fn radius(&self, i: usize) -> f32 {
        self.sizes.get(i).copied().unwrap_or(6.0).clamp(5.0, 26.0)
    }
}

fn resolve_edges(nodes: &[Symbol], edges: &[Relationship]) -> Vec<(usize, usize)> {
    let index: HashMap<&SymbolId, usize> =
        nodes.iter().enumerate().map(|(i, n)| (&n.id, i)).collect();
    edges
        .iter()
        .filter_map(|e| Some((*index.get(&e.source_id)?, *index.get(&e.target_id)?)))
        .collect()
}

/// Hierarchical (Sugiyama-style) layered layout for a directed dependency
/// graph. Cycles are condensed into strongly-connected components first, so
/// the layering is always well-defined.
///
/// Layer 0 holds nodes with no inbound dependencies (roots); each next layer
/// contains nodes whose dependencies are above it. Within a layer, nodes are
/// ordered by the barycenter of their parents to reduce edge crossings.
pub fn layered_layout(n: usize, edges: &[(usize, usize)]) -> Vec<(f32, f32)> {
    if n == 0 {
        return Vec::new();
    }

    let mut g: DiGraph<usize, ()> = DiGraph::with_capacity(n, edges.len());
    for i in 0..n {
        g.add_node(i);
    }
    for &(a, b) in edges {
        if a < n && b < n && a != b {
            g.add_edge(NodeIndex::new(a), NodeIndex::new(b), ());
        }
    }

    // Condense cycles: each SCC becomes one node in a DAG.
    let sccs = tarjan_scc(&g);
    let mut component_of = vec![0usize; n];
    for (cid, comp) in sccs.iter().enumerate() {
        for &node in comp {
            component_of[node.index()] = cid;
        }
    }
    let m = sccs.len();

    // Condensation adjacency + in-degrees.
    let mut cadj: Vec<Vec<usize>> = vec![Vec::new(); m];
    let mut indeg = vec![0usize; m];
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    for &(a, b) in edges {
        if a < n && b < n {
            let (ca, cb) = (component_of[a], component_of[b]);
            if ca != cb && seen.insert((ca, cb)) {
                cadj[ca].push(cb);
                indeg[cb] += 1;
            }
        }
    }

    // Topological order of the condensation, then longest-path layering.
    let mut queue: VecDeque<usize> = (0..m).filter(|&i| indeg[i] == 0).collect();
    let mut order: Vec<usize> = Vec::with_capacity(m);
    while let Some(u) = queue.pop_front() {
        order.push(u);
        for &v in &cadj[u] {
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push_back(v);
            }
        }
    }
    for u in 0..m {
        if !order.contains(&u) {
            order.push(u);
        }
    }
    let mut layer = vec![0usize; m];
    for &u in &order {
        for &v in &cadj[u] {
            if layer[u] + 1 > layer[v] {
                layer[v] = layer[u] + 1;
            }
        }
    }

    // Group nodes by layer.
    let max_layer = layer.iter().copied().max().unwrap_or(0);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_layer + 1];
    for i in 0..n {
        layers[layer[component_of[i]]].push(i);
    }

    // Parent lists for barycenter ordering (edges that go down a layer).
    let mut parents: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in edges {
        if a < n && b < n && layer[component_of[a]] < layer[component_of[b]] {
            parents[b].push(a);
        }
    }

    let h_gap = 110.0f32;
    let v_gap = 130.0f32;
    let total_h = max_layer as f32 * v_gap;

    let mut positions = vec![(0.0f32, 0.0f32); n];
    let mut x_of = vec![0.0f32; n];
    for (li, nodes) in layers.iter_mut().enumerate() {
        nodes.sort_by(|&a, &b| {
            let ba = barycenter(&parents[a], &x_of);
            let bb = barycenter(&parents[b], &x_of);
            ba.partial_cmp(&bb)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(&b))
        });
        let width = (nodes.len().saturating_sub(1)) as f32 * h_gap;
        let y = li as f32 * v_gap - total_h / 2.0;
        for (k, &node) in nodes.iter().enumerate() {
            let x = k as f32 * h_gap - width / 2.0;
            x_of[node] = x;
            positions[node] = (x, y);
        }
    }

    for p in &mut positions {
        if !p.0.is_finite() || !p.1.is_finite() {
            *p = (0.0, 0.0);
        }
    }
    positions
}

fn barycenter(parents: &[usize], x_of: &[f32]) -> f32 {
    if parents.is_empty() {
        0.0
    } else {
        parents.iter().map(|&p| x_of[p]).sum::<f32>() / parents.len() as f32
    }
}

/// PageRank-based node sizing (small graphs: simple power iteration).
fn compute_pagerank(n: usize, edges: &[(usize, usize)]) -> Vec<f32> {
    if n == 0 {
        return Vec::new();
    }
    let mut rank = vec![1.0f32 / n as f32; n];
    let mut out_deg = vec![0usize; n];
    for &(a, _) in edges {
        if a < n {
            out_deg[a] += 1;
        }
    }
    let d = 0.85f32;
    let mut next = vec![0.0f32; n];
    for _ in 0..30 {
        for v in next.iter_mut() {
            *v = (1.0 - d) / n as f32;
        }
        for &(a, b) in edges {
            if a < n && b < n && out_deg[a] > 0 {
                next[b] += d * rank[a] / out_deg[a] as f32;
            }
        }
        std::mem::swap(&mut rank, &mut next);
    }
    rank
}

/// Derive a coarse module key from a file path: the first two path segments,
/// skipping a leading `src/`, normalized to `/`.
pub fn module_key(path: &str) -> String {
    let norm = path.replace('\\', "/");
    let mut segs: Vec<&str> = norm
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    if segs.first() == Some(&"src") {
        segs.remove(0);
    }
    match segs.len() {
        0 => "(root)".to_string(),
        1 => segs[0].to_string(),
        _ => format!("{}/{}", segs[0], segs[1]),
    }
}

/// Keep only the strongest `max_edges` aggregated edges (by weight).
fn top_edges(weights: HashMap<(String, String), u32>, max_edges: usize) -> Vec<Relationship> {
    use crate::model::RelationshipKind;
    let mut v: Vec<((String, String), u32)> = weights.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(max_edges.max(1));
    v.into_iter()
        .enumerate()
        .map(|(i, ((s, t), _w))| {
            Relationship::new(
                i as u64,
                SymbolId(s),
                SymbolId(t),
                RelationshipKind::Other("aggregated".to_string()),
            )
        })
        .collect()
}

/// Build a coarse module-level overview (first 1-2 path segments). This is the
/// default "whole graph" view: tens of readable super-nodes instead of a
/// 10k-node hairball.
pub fn build_module_overview(graph: &crate::CodeGraph, max_modules: usize) -> Option<GraphView> {
    use crate::model::SymbolKind;
    use std::collections::HashSet;

    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut module_of: HashMap<&SymbolId, String> = HashMap::new();
    for s in graph.symbols() {
        let m = module_key(&s.file);
        *counts.entry(m.clone()).or_default() += 1;
        module_of.insert(&s.id, m);
    }
    if counts.is_empty() {
        return None;
    }

    let mut modules: Vec<(String, usize)> = counts.into_iter().collect();
    modules.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    modules.truncate(max_modules.max(1));
    let keep: HashSet<&str> = modules.iter().map(|(m, _)| m.as_str()).collect();

    let mut weights: HashMap<(String, String), u32> = HashMap::new();
    for r in graph.relationships() {
        let (Some(sm), Some(tm)) = (module_of.get(&r.source_id), module_of.get(&r.target_id))
        else {
            continue;
        };
        if sm != tm && keep.contains(sm.as_str()) && keep.contains(tm.as_str()) {
            *weights.entry((sm.clone(), tm.clone())).or_default() += 1;
        }
    }
    let edges = top_edges(weights, modules.len() * 4);

    let max_count = modules.first().map(|(_, c)| *c).unwrap_or(1).max(1) as f32;
    let mut symbols = Vec::with_capacity(modules.len());
    let mut sizes = Vec::with_capacity(modules.len());
    for (module, count) in &modules {
        // Keep the parent segment so `app/admin` and `components/admin` do not
        // both render as "admin".
        let name = module.clone();
        let mut sym = Symbol::new(
            SymbolId(module.clone()),
            name,
            SymbolKind::Module,
            module.clone(),
        );
        sym.doc = Some(format!("{count} symbols"));
        symbols.push(sym);
        let t = (*count as f32 / max_count).sqrt();
        sizes.push(8.0 + t * 18.0);
    }

    Some(GraphView::new_with_sizes(symbols, edges, None, sizes))
}

/// Build a file-level overview, optionally restricted to files under a module
/// prefix (drill-down). Edges are filtered to the strongest few.
pub fn build_file_overview(
    graph: &crate::CodeGraph,
    max_files: usize,
    module_filter: Option<&str>,
) -> Option<GraphView> {
    use crate::model::SymbolKind;
    use std::collections::HashSet;

    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut file_of: HashMap<&SymbolId, &str> = HashMap::new();
    for s in graph.symbols() {
        if let Some(prefix) = module_filter {
            if module_key(&s.file) != prefix {
                continue;
            }
        }
        *counts.entry(&s.file).or_default() += 1;
        file_of.insert(&s.id, &s.file);
    }
    if counts.is_empty() {
        return None;
    }

    let mut files: Vec<(&str, usize)> = counts.into_iter().collect();
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    files.truncate(max_files.max(1));
    let keep: HashSet<&str> = files.iter().map(|(f, _)| *f).collect();

    let max_count = files.first().map(|(_, c)| *c).unwrap_or(1).max(1) as f32;
    let mut symbols = Vec::with_capacity(files.len());
    let mut sizes = Vec::with_capacity(files.len());
    for (file, count) in &files {
        let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
        let mut sym = Symbol::new(
            SymbolId((*file).to_string()),
            base,
            SymbolKind::Module,
            (*file).to_string(),
        );
        sym.doc = Some(format!("{count} symbols"));
        symbols.push(sym);
        let t = (*count as f32 / max_count).sqrt();
        sizes.push(6.0 + t * 16.0);
    }

    let mut weights: HashMap<(String, String), u32> = HashMap::new();
    for r in graph.relationships() {
        let (Some(&sf), Some(&tf)) = (file_of.get(&r.source_id), file_of.get(&r.target_id)) else {
            continue;
        };
        if sf != tf && keep.contains(sf) && keep.contains(tf) {
            *weights.entry((sf.to_string(), tf.to_string())).or_default() += 1;
        }
    }
    let edges = top_edges(weights, files.len() * 3);

    Some(GraphView::new_with_sizes(symbols, edges, None, sizes))
}

/// Build a visualizer for `focus`'s 1-hop ego-network (focus + neighbors,
/// capped at `max_nodes`).
pub fn build_ego_view(
    graph: &crate::CodeGraph,
    focus: &SymbolId,
    max_nodes: usize,
) -> Option<GraphView> {
    use crate::model::EdgeDirection;
    use std::collections::HashSet;

    graph.symbol(focus.clone())?;

    let mut ids: Vec<SymbolId> = vec![focus.clone()];
    let mut seen: HashSet<SymbolId> = HashSet::new();
    seen.insert(focus.clone());

    for rel in graph.relationships_for(focus.clone(), EdgeDirection::Outgoing) {
        if ids.len() >= max_nodes {
            break;
        }
        if seen.insert(rel.target_id.clone()) {
            ids.push(rel.target_id.clone());
        }
    }
    for rel in graph.relationships_for(focus.clone(), EdgeDirection::Incoming) {
        if ids.len() >= max_nodes {
            break;
        }
        if seen.insert(rel.source_id.clone()) {
            ids.push(rel.source_id.clone());
        }
    }

    let symbols: Vec<Symbol> = ids
        .iter()
        .filter_map(|id| graph.symbol(id.clone()).cloned())
        .collect();
    if symbols.is_empty() {
        return None;
    }

    let mut edges = Vec::new();
    for id in &ids {
        for rel in graph.relationships_for(id.clone(), EdgeDirection::Outgoing) {
            if ids.contains(&rel.target_id) {
                edges.push(rel.clone());
            }
        }
    }

    Some(GraphView::new(symbols, edges, Some(focus.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layered_layout_produces_finite_positions() {
        let edges = vec![(0, 1), (1, 2), (2, 0), (3, 1)];
        let pos = layered_layout(4, &edges);
        assert_eq!(pos.len(), 4);
        assert!(pos.iter().all(|p| p.0.is_finite() && p.1.is_finite()));
    }

    #[test]
    fn layered_layout_handles_empty() {
        assert!(layered_layout(0, &[]).is_empty());
    }

    #[test]
    fn pagerank_sums_to_about_one() {
        let edges = vec![(0, 1), (1, 2), (2, 0)];
        let rank = compute_pagerank(3, &edges);
        let total: f32 = rank.iter().sum();
        assert!((total - 1.0).abs() < 0.05, "total was {total}");
    }

    #[test]
    fn single_node_layout_is_finite() {
        let pos = layered_layout(1, &[]);
        assert_eq!(pos.len(), 1);
        assert!(pos[0].0.is_finite() && pos[0].1.is_finite());
    }

    #[test]
    fn layered_layout_stacks_dependency_below_source() {
        // 0 -> 1 means 1 depends on 0, so 1 must sit in a lower layer (larger y).
        let pos = layered_layout(2, &[(0, 1)]);
        assert!(pos[1].1 > pos[0].1, "target should be below source");
    }

    #[test]
    fn module_key_skips_src_and_uses_two_segments() {
        assert_eq!(module_key("src/components/ui/button.tsx"), "components/ui");
        assert_eq!(module_key("app/(admin)/x.ts"), "app/(admin)");
        assert_eq!(module_key("lib.rs"), "lib.rs");
        assert_eq!(module_key("src/lib.rs"), "lib.rs");
        assert_eq!(module_key("a\\b\\c.ts"), "a/b");
    }

    #[test]
    fn top_edges_keeps_only_the_strongest() {
        let mut w: HashMap<(String, String), u32> = HashMap::new();
        w.insert(("a".into(), "b".into()), 1);
        w.insert(("c".into(), "d".into()), 9);
        w.insert(("e".into(), "f".into()), 5);
        let edges = top_edges(w, 2);
        assert_eq!(edges.len(), 2);
        let mut pairs: Vec<(String, String)> = edges
            .iter()
            .map(|e| (e.source_id.0.clone(), e.target_id.0.clone()))
            .collect();
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("c".to_string(), "d".to_string()),
                ("e".to_string(), "f".to_string())
            ]
        );
    }
}
