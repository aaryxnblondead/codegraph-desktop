//! Dependency Structure Matrix (DSM) construction.

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Matrix {
    pub units: Vec<String>,
    /// `cells[i][j]` = weight of edges from `units[i]` to `units[j]`.
    pub cells: Vec<Vec<usize>>,
}

impl Matrix {
    pub fn cell(&self, from: usize, to: usize) -> usize {
        self.cells
            .get(from)
            .and_then(|row| row.get(to))
            .copied()
            .unwrap_or(0)
    }

    pub fn total(&self) -> usize {
        self.cells.iter().flatten().sum()
    }
}

/// Build a matrix for `units` from aggregated cross edges.
pub fn build(cross: &HashMap<(String, String), usize>, units: &[String]) -> Matrix {
    let index: HashMap<&str, usize> = units
        .iter()
        .enumerate()
        .map(|(i, u)| (u.as_str(), i))
        .collect();
    let mut cells = vec![vec![0usize; units.len()]; units.len()];
    for ((a, b), w) in cross {
        if let (Some(&i), Some(&j)) = (index.get(a.as_str()), index.get(b.as_str())) {
            cells[i][j] += w;
        }
    }
    Matrix {
        units: units.to_vec(),
        cells,
    }
}

/// Order units so dependencies mostly sit above the diagonal (Kahn layering;
/// cycles fall back to name order).
pub fn topological_order(
    cross: &HashMap<(String, String), usize>,
    units: &[String],
) -> Vec<String> {
    use std::collections::{BTreeSet, VecDeque};
    let index: HashMap<&str, usize> = units
        .iter()
        .enumerate()
        .map(|(i, u)| (u.as_str(), i))
        .collect();
    let mut adj: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); units.len()];
    let mut indeg = vec![0usize; units.len()];
    for (a, b) in cross.keys() {
        if let (Some(&i), Some(&j)) = (index.get(a.as_str()), index.get(b.as_str())) {
            if i != j && adj[i].insert(j) {
                indeg[j] += 1;
            }
        }
    }
    let mut queue: VecDeque<usize> = (0..units.len()).filter(|&i| indeg[i] == 0).collect();
    let mut order = Vec::new();
    while let Some(u) = queue.pop_front() {
        order.push(u);
        let next: Vec<usize> = adj[u].iter().copied().collect();
        for v in next {
            indeg[v] = indeg[v].saturating_sub(1);
            if indeg[v] == 0 {
                queue.push_back(v);
            }
        }
    }
    // Append any nodes left in cycles.
    let mut seen: BTreeSet<usize> = order.iter().copied().collect();
    for i in 0..units.len() {
        if seen.insert(i) {
            order.push(i);
        }
    }
    order.into_iter().map(|i| units[i].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units() -> Vec<String> {
        vec!["a".into(), "b".into(), "c".into()]
    }

    #[test]
    fn cells_and_totals() {
        let mut cross = HashMap::new();
        cross.insert(("a".to_string(), "b".to_string()), 3);
        cross.insert(("b".to_string(), "c".to_string()), 2);
        let m = build(&cross, &units());
        assert_eq!(m.cell(0, 1), 3);
        assert_eq!(m.cell(1, 2), 2);
        assert_eq!(m.total(), 5);
    }

    #[test]
    fn topological_order_places_sources_first() {
        let mut cross = HashMap::new();
        cross.insert(("a".to_string(), "b".to_string()), 1);
        cross.insert(("b".to_string(), "c".to_string()), 1);
        let order = topological_order(&cross, &units());
        assert_eq!(
            order,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn cycle_falls_back_without_panicking() {
        let mut cross = HashMap::new();
        cross.insert(("a".to_string(), "b".to_string()), 1);
        cross.insert(("b".to_string(), "a".to_string()), 1);
        let order = topological_order(&cross, &units());
        assert_eq!(order.len(), 3);
    }
}
