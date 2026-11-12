//! Change impact analysis: transitive closures and shortest dependency paths.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::graph::CodeGraph;
use crate::model::{EdgeDirection, RelationshipKind, SymbolId};

#[derive(Debug, Clone)]
pub struct Impact {
    pub start: SymbolId,
    /// (node, depth) for every reachable node.
    pub nodes: Vec<(SymbolId, usize)>,
    /// Count of nodes at each depth.
    pub by_depth: BTreeMap<usize, usize>,
    pub truncated: bool,
}

/// Reverse closure: who depends on `start` (transitively).
pub fn reverse_closure(
    graph: &CodeGraph,
    start: &SymbolId,
    kinds: &[RelationshipKind],
    min_confidence: f32,
    max_depth: usize,
    cap: usize,
) -> Impact {
    closure(
        graph,
        start,
        EdgeDirection::Incoming,
        kinds,
        min_confidence,
        max_depth,
        cap,
    )
}

/// Forward closure: what `start` depends on (transitively).
pub fn forward_closure(
    graph: &CodeGraph,
    start: &SymbolId,
    kinds: &[RelationshipKind],
    min_confidence: f32,
    max_depth: usize,
    cap: usize,
) -> Impact {
    closure(
        graph,
        start,
        EdgeDirection::Outgoing,
        kinds,
        min_confidence,
        max_depth,
        cap,
    )
}

fn closure(
    graph: &CodeGraph,
    start: &SymbolId,
    direction: EdgeDirection,
    kinds: &[RelationshipKind],
    min_confidence: f32,
    max_depth: usize,
    cap: usize,
) -> Impact {
    let mut seen: HashSet<SymbolId> = HashSet::new();
    seen.insert(start.clone());
    let mut queue: VecDeque<(SymbolId, usize)> = VecDeque::new();
    queue.push_back((start.clone(), 0));
    let mut nodes = Vec::new();
    let mut by_depth: BTreeMap<usize, usize> = BTreeMap::new();
    let mut truncated = false;

    while let Some((id, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        for rel in graph.relationships_for_filtered(id.clone(), direction, kinds, min_confidence) {
            let next = match direction {
                EdgeDirection::Outgoing => &rel.target_id,
                EdgeDirection::Incoming => &rel.source_id,
            };
            if seen.insert(next.clone()) {
                if nodes.len() >= cap {
                    truncated = true;
                    continue;
                }
                nodes.push((next.clone(), depth + 1));
                *by_depth.entry(depth + 1).or_default() += 1;
                queue.push_back((next.clone(), depth + 1));
            }
        }
    }
    Impact {
        start: start.clone(),
        nodes,
        by_depth,
        truncated,
    }
}

/// Shortest dependency path from `from` to `to` (BFS), or `None`.
pub fn shortest_path(
    graph: &CodeGraph,
    from: &SymbolId,
    to: &SymbolId,
    kinds: &[RelationshipKind],
    min_confidence: f32,
) -> Option<Vec<SymbolId>> {
    let mut prev: HashMap<SymbolId, SymbolId> = HashMap::new();
    let mut seen: HashSet<SymbolId> = HashSet::new();
    seen.insert(from.clone());
    let mut queue: VecDeque<SymbolId> = VecDeque::new();
    queue.push_back(from.clone());
    while let Some(id) = queue.pop_front() {
        if &id == to {
            let mut path = vec![id];
            let mut cur = path[0].clone();
            while let Some(p) = prev.get(&cur) {
                path.push(p.clone());
                cur = p.clone();
            }
            path.reverse();
            return Some(path);
        }
        for rel in graph.relationships_for_filtered(
            id.clone(),
            EdgeDirection::Outgoing,
            kinds,
            min_confidence,
        ) {
            if seen.insert(rel.target_id.clone()) {
                prev.insert(rel.target_id.clone(), id.clone());
                queue.push_back(rel.target_id.clone());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Relationship, Symbol};

    fn sym(id: &str) -> Symbol {
        Symbol::new(SymbolId(id.into()), id, Default::default(), "f.ts")
    }

    fn graph() -> CodeGraph {
        // a -> b -> c -> d
        let rels = vec![(1, "a", "b"), (2, "b", "c"), (3, "c", "d")]
            .into_iter()
            .map(|(id, s, t)| {
                Relationship::new(
                    id,
                    SymbolId(s.into()),
                    SymbolId(t.into()),
                    RelationshipKind::Calls,
                )
            })
            .collect();
        CodeGraph::new(vec![sym("a"), sym("b"), sym("c"), sym("d")], rels).unwrap()
    }

    #[test]
    fn forward_and_reverse_closures() {
        let g = graph();
        let kinds = [RelationshipKind::Calls];
        let fwd = forward_closure(&g, &SymbolId("a".into()), &kinds, 0.0, 6, 1000);
        assert_eq!(fwd.nodes.len(), 3);
        assert_eq!(fwd.by_depth.get(&1), Some(&1));
        assert_eq!(fwd.by_depth.get(&3), Some(&1));

        let rev = reverse_closure(&g, &SymbolId("d".into()), &kinds, 0.0, 6, 1000);
        assert_eq!(rev.nodes.len(), 3);
    }

    #[test]
    fn shortest_path_finds_route() {
        let g = graph();
        let kinds = [RelationshipKind::Calls];
        let path = shortest_path(
            &g,
            &SymbolId("a".into()),
            &SymbolId("d".into()),
            &kinds,
            0.0,
        )
        .expect("path");
        let names: Vec<&str> = path.iter().map(|s| s.0.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c", "d"]);
        assert!(shortest_path(
            &g,
            &SymbolId("d".into()),
            &SymbolId("a".into()),
            &kinds,
            0.0
        )
        .is_none());
    }

    #[test]
    fn depth_limit_and_cap() {
        let g = graph();
        let kinds = [RelationshipKind::Calls];
        let fwd = forward_closure(&g, &SymbolId("a".into()), &kinds, 0.0, 1, 1000);
        assert_eq!(fwd.nodes.len(), 1);
        let capped = forward_closure(&g, &SymbolId("a".into()), &kinds, 0.0, 6, 1);
        assert!(capped.truncated);
        assert_eq!(capped.nodes.len(), 1);
    }
}
