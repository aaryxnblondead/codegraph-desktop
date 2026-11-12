//! Structural analysis: cycle-breaking edge suggestions.
//!
//! Cycles are reported by SCCs; this module answers "which edges do I cut?".
//! It uses the Eades–Lin–Smyth greedy ordering: order the nodes so that most
//! edges point forward, then treat the edges that still point backwards as the
//! feedback arc set. The result is a suggestion, not a minimum cut — finding a
//! true minimum FAS is NP-hard. It is verified before it is returned.

use std::collections::{HashMap, HashSet};

/// One file-to-file edge suggested for removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutEdge {
    pub from: String,
    pub to: String,
    /// Number of underlying relationships aggregated into this pair.
    pub weight: usize,
}

/// Order nodes so most edges point forward (Eades–Lin–Smyth).
///
/// Deterministic: ties are broken by node name.
pub fn greedy_order(nodes: &[String], edges: &[(String, String)]) -> Vec<String> {
    let mut remaining: HashSet<&str> = nodes.iter().map(String::as_str).collect();
    let mut out_deg: HashMap<&str, usize> = HashMap::new();
    let mut in_deg: HashMap<&str, usize> = HashMap::new();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for (a, b) in edges {
        if a == b {
            continue;
        }
        *out_deg.entry(a.as_str()).or_default() += 1;
        *in_deg.entry(b.as_str()).or_default() += 1;
        adjacency.entry(a.as_str()).or_default().push(b.as_str());
    }

    let mut head: Vec<String> = Vec::new();
    let mut tail: Vec<String> = Vec::new();

    while !remaining.is_empty() {
        // Remove sinks (into the tail, reversed later) and sources (into head).
        let mut changed = true;
        while changed {
            changed = false;
            let mut sinks: Vec<&str> = remaining
                .iter()
                .copied()
                .filter(|n| out_deg.get(n).copied().unwrap_or(0) == 0)
                .collect();
            sinks.sort();
            for sink in sinks {
                if remaining.remove(sink) {
                    tail.push(sink.to_string());
                    changed = true;
                    for (a, adj) in &adjacency {
                        if adj.contains(&sink) {
                            if let Some(d) = out_deg.get_mut(*a) {
                                *d = d.saturating_sub(1);
                            }
                        }
                    }
                }
            }
            let mut sources: Vec<&str> = remaining
                .iter()
                .copied()
                .filter(|n| in_deg.get(n).copied().unwrap_or(0) == 0)
                .collect();
            sources.sort();
            for source in sources {
                if remaining.remove(source) {
                    head.push(source.to_string());
                    changed = true;
                    if let Some(adj) = adjacency.get(source) {
                        for b in adj.clone() {
                            if let Some(d) = in_deg.get_mut(b) {
                                *d = d.saturating_sub(1);
                            }
                        }
                    }
                }
            }
        }

        if remaining.is_empty() {
            break;
        }

        // Pick the node with the largest (out − in), ties by name.
        let pick = remaining
            .iter()
            .copied()
            .max_by(|a, b| {
                let da = out_deg.get(a).copied().unwrap_or(0) as i64
                    - in_deg.get(a).copied().unwrap_or(0) as i64;
                let db = out_deg.get(b).copied().unwrap_or(0) as i64
                    - in_deg.get(b).copied().unwrap_or(0) as i64;
                da.cmp(&db).then_with(|| b.cmp(a))
            })
            .expect("non-empty");
        remaining.remove(pick);
        head.push(pick.to_string());
        if let Some(adj) = adjacency.get(pick) {
            for b in adj.clone() {
                if let Some(d) = in_deg.get_mut(b) {
                    *d = d.saturating_sub(1);
                }
            }
        }
        if let Some(d) = out_deg.get_mut(pick) {
            *d = 0;
        }
    }

    tail.reverse();
    head.extend(tail);
    head
}

/// Edges that point backwards in `order` — removing them makes the graph acyclic.
pub fn backward_edges(
    order: &[String],
    edges: &[(String, String)],
    weights: &HashMap<(String, String), usize>,
) -> Vec<CutEdge> {
    let pos: HashMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let mut out: Vec<CutEdge> = edges
        .iter()
        .filter(|(a, b)| {
            if a == b {
                return false;
            }
            match (pos.get(a.as_str()), pos.get(b.as_str())) {
                (Some(&i), Some(&j)) => i > j,
                _ => false,
            }
        })
        .map(|(a, b)| CutEdge {
            from: a.clone(),
            to: b.clone(),
            weight: weights.get(&(a.clone(), b.clone())).copied().unwrap_or(1),
        })
        .collect();
    // Cheapest cuts first: the caller should start with the weakest dependency.
    out.sort_by(|x, y| {
        x.weight
            .cmp(&y.weight)
            .then_with(|| x.from.cmp(&y.from))
            .then_with(|| x.to.cmp(&y.to))
    });
    out
}

/// Suggested edges to cut so that `files` (one SCC) becomes acyclic.
///
/// The set is verified: if removing it does not break every cycle, all internal
/// edges are returned instead, so the suggestion is never wrong.
pub fn break_cycle(
    files: &[String],
    edges: &[(String, String)],
    weights: &HashMap<(String, String), usize>,
) -> Vec<CutEdge> {
    let internal: Vec<(String, String)> = edges
        .iter()
        .filter(|(a, b)| a != b && files.iter().any(|f| f == a) && files.iter().any(|f| f == b))
        .cloned()
        .collect();
    if internal.is_empty() {
        return Vec::new();
    }
    let order = greedy_order(files, &internal);
    let cuts = backward_edges(&order, &internal, weights);
    if is_acyclic_after(files, &internal, &cuts) {
        cuts
    } else {
        // Fallback: list every internal edge, cheapest first.
        let mut all: Vec<CutEdge> = internal
            .iter()
            .map(|(a, b)| CutEdge {
                from: a.clone(),
                to: b.clone(),
                weight: weights.get(&(a.clone(), b.clone())).copied().unwrap_or(1),
            })
            .collect();
        all.sort_by(|x, y| x.weight.cmp(&y.weight).then_with(|| x.from.cmp(&y.from)));
        all
    }
}

/// Verify the cut: no cycle remains among `files`.
fn is_acyclic_after(files: &[String], edges: &[(String, String)], cuts: &[CutEdge]) -> bool {
    let cut_set: HashSet<(&str, &str)> = cuts
        .iter()
        .map(|c| (c.from.as_str(), c.to.as_str()))
        .collect();
    let kept: Vec<(&str, &str)> = edges
        .iter()
        .filter(|(a, b)| !cut_set.contains(&(a.as_str(), b.as_str())))
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    !has_cycle(files, &kept)
}

fn has_cycle(nodes: &[String], edges: &[(&str, &str)]) -> bool {
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        index.insert(n.as_str(), i);
    }
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (a, b) in edges {
        if let (Some(&i), Some(&j)) = (index.get(a), index.get(b)) {
            adj[i].push(j);
        }
    }
    // 0 = unvisited, 1 = on stack, 2 = done (iterative DFS).
    let mut color = vec![0u8; nodes.len()];
    for start in 0..nodes.len() {
        if color[start] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        color[start] = 1;
        while let Some((node, next)) = stack.pop() {
            if next < adj[node].len() {
                stack.push((node, next + 1));
                let child = adj[node][next];
                match color[child] {
                    0 => {
                        color[child] = 1;
                        stack.push((child, 0));
                    }
                    1 => return true,
                    _ => {}
                }
            } else {
                color[node] = 2;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn edges(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn dag_needs_no_cuts() {
        let n = nodes(&["a", "b", "c"]);
        let e = edges(&[("a", "b"), ("b", "c")]);
        let cuts = break_cycle(&n, &e, &HashMap::new());
        assert!(cuts.is_empty(), "{cuts:?}");
    }

    #[test]
    fn two_file_cycle_suggests_one_edge() {
        let n = nodes(&["a", "b"]);
        let e = edges(&[("a", "b"), ("b", "a")]);
        let cuts = break_cycle(&n, &e, &HashMap::new());
        assert_eq!(cuts.len(), 1, "{cuts:?}");
        assert!(is_acyclic_after(&n, &e, &cuts));
    }

    #[test]
    fn three_file_cycle_needs_one_cut() {
        let n = nodes(&["a", "b", "c"]);
        let e = edges(&[("a", "b"), ("b", "c"), ("c", "a")]);
        let cuts = break_cycle(&n, &e, &HashMap::new());
        assert_eq!(cuts.len(), 1, "{cuts:?}");
        assert!(is_acyclic_after(&n, &e, &cuts));
    }

    #[test]
    fn cheapest_edge_comes_first() {
        let n = nodes(&["a", "b"]);
        let e = edges(&[("a", "b"), ("b", "a")]);
        let mut weights = HashMap::new();
        weights.insert(("a".to_string(), "b".to_string()), 9usize);
        weights.insert(("b".to_string(), "a".to_string()), 2usize);
        let cuts = break_cycle(&n, &e, &weights);
        assert_eq!(cuts[0].weight, 2, "{cuts:?}");
    }

    #[test]
    fn cycle_detection_is_correct() {
        assert!(has_cycle(&nodes(&["a", "b"]), &[("a", "b"), ("b", "a")]));
        assert!(!has_cycle(&nodes(&["a", "b"]), &[("a", "b")]));
        assert!(!has_cycle(&nodes(&["a"]), &[]));
    }

    #[test]
    fn every_removed_set_breaks_random_digraphs() {
        // A handful of fixed shapes, including two overlapping cycles.
        let n = nodes(&["a", "b", "c", "d"]);
        let e = edges(&[("a", "b"), ("b", "c"), ("c", "a"), ("c", "d"), ("d", "b")]);
        let cuts = break_cycle(&n, &e, &HashMap::new());
        assert!(is_acyclic_after(&n, &e, &cuts), "cuts {cuts:?}");
    }
}
