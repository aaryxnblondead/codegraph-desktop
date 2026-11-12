//! In-memory code graph with lookup indexes.
//!
//! `Storage` loads raw rows; `CodeGraph` owns them and builds `HashMap`
//! indexes so symbol/relationship lookups are O(1) and the UI never touches
//! storage after loading.

use std::collections::HashMap;

use crate::error::CodegraphError;
use crate::model::{EdgeDirection, FileInfo, Relationship, RelationshipKind, Symbol, SymbolId};

/// An in-memory code graph: symbols (nodes) plus directed relationships
/// (edges), indexed by id and by source/target for fast traversal.
#[derive(Debug, Clone, Default)]
pub struct CodeGraph {
    symbols: Vec<Symbol>,
    by_id: HashMap<SymbolId, Symbol>,
    relationships: Vec<Relationship>,
    /// Outgoing edges keyed by source symbol id.
    outgoing: HashMap<SymbolId, Vec<Relationship>>,
    /// Incoming edges keyed by target symbol id.
    incoming: HashMap<SymbolId, Vec<Relationship>>,
    /// Per-file information keyed by relative path.
    files: HashMap<String, FileInfo>,
}

impl CodeGraph {
    /// Build a graph from symbols and relationships, indexing both.
    pub fn new(
        symbols: Vec<Symbol>,
        relationships: Vec<Relationship>,
    ) -> Result<Self, CodegraphError> {
        Self::new_with_files(symbols, relationships, HashMap::new())
    }

    /// Build a graph with an explicit per-file map.
    pub fn new_with_files(
        symbols: Vec<Symbol>,
        relationships: Vec<Relationship>,
        files: HashMap<String, FileInfo>,
    ) -> Result<Self, CodegraphError> {
        let mut by_id = HashMap::with_capacity(symbols.len());
        for symbol in &symbols {
            if by_id.insert(symbol.id.clone(), symbol.clone()).is_some() {
                return Err(CodegraphError::DuplicateSymbolId {
                    id: symbol.id.clone(),
                });
            }
        }

        let mut outgoing: HashMap<SymbolId, Vec<Relationship>> = HashMap::new();
        let mut incoming: HashMap<SymbolId, Vec<Relationship>> = HashMap::new();
        for rel in &relationships {
            outgoing
                .entry(rel.source_id.clone())
                .or_default()
                .push(rel.clone());
            incoming
                .entry(rel.target_id.clone())
                .or_default()
                .push(rel.clone());
        }

        Ok(Self {
            symbols,
            by_id,
            relationships,
            outgoing,
            incoming,
            files,
        })
    }

    pub fn symbol_count(&self) -> usize {
        self.symbols.len()
    }

    pub fn relationship_count(&self) -> usize {
        self.relationships.len()
    }

    pub fn relationships(&self) -> &[Relationship] {
        &self.relationships
    }

    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    pub fn files(&self) -> &HashMap<String, FileInfo> {
        &self.files
    }

    pub fn file(&self, path: &str) -> Option<&FileInfo> {
        self.files.get(path)
    }

    pub fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.by_id.get(&id)
    }

    /// Symbols whose name, file, or signature contains `query`.
    pub fn find(&self, query: &str) -> Vec<&Symbol> {
        let q = query.to_lowercase();
        if q.is_empty() {
            return self.symbols.iter().collect();
        }
        self.symbols
            .iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&q)
                    || s.file.to_lowercase().contains(&q)
                    || s.qualified_name.to_lowercase().contains(&q)
                    || s.signature
                        .as_deref()
                        .is_some_and(|sig| sig.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// Relationships touching `id` in the given direction.
    pub fn relationships_for(&self, id: SymbolId, direction: EdgeDirection) -> &[Relationship] {
        match direction {
            EdgeDirection::Outgoing => self.outgoing.get(&id),
            EdgeDirection::Incoming => self.incoming.get(&id),
        }
        .map(Vec::as_slice)
        .unwrap_or(&[])
    }

    /// Relationships filtered by kind and minimum confidence.
    ///
    /// An empty `kinds` slice means "all kinds". Structural `contains` edges
    /// are excluded when the caller passes explicit kinds.
    pub fn relationships_for_filtered(
        &self,
        id: SymbolId,
        direction: EdgeDirection,
        kinds: &[RelationshipKind],
        min_confidence: f32,
    ) -> Vec<&Relationship> {
        self.relationships_for(id, direction)
            .iter()
            .filter(|r| r.confidence >= min_confidence)
            .filter(|r| kinds.is_empty() || kinds.contains(&r.kind))
            .collect()
    }

    pub fn neighbors_out(&self, id: SymbolId) -> Vec<&Symbol> {
        self.outgoing
            .get(&id)
            .into_iter()
            .flat_map(|rels| rels.iter().map(|r| r.target_id.clone()))
            .filter_map(|id| self.by_id.get(&id))
            .collect()
    }

    pub fn neighbors_in(&self, id: SymbolId) -> Vec<&Symbol> {
        self.incoming
            .get(&id)
            .into_iter()
            .flat_map(|rels| rels.iter().map(|r| r.source_id.clone()))
            .filter_map(|id| self.by_id.get(&id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileInfo, FileStatus, RelationshipKind};

    fn sym(id: &str, name: &str) -> Symbol {
        Symbol::new(SymbolId(id.into()), name, Default::default(), "src/lib.rs")
    }

    fn rel(id: u64, from: &str, to: &str) -> Relationship {
        Relationship::new(
            id,
            SymbolId(from.into()),
            SymbolId(to.into()),
            RelationshipKind::Calls,
        )
    }

    #[test]
    fn build_and_lookup() {
        let graph = CodeGraph::new(
            vec![sym("1", "alpha"), sym("2", "beta")],
            vec![rel(1, "1", "2")],
        )
        .expect("valid graph");

        assert_eq!(graph.symbol_count(), 2);
        assert_eq!(graph.relationship_count(), 1);
        assert_eq!(
            graph.symbol(SymbolId("1".into())).map(|s| s.name.as_str()),
            Some("alpha")
        );
        assert_eq!(
            graph
                .relationships_for(SymbolId("1".into()), EdgeDirection::Outgoing)
                .len(),
            1
        );
        assert!(graph
            .relationships_for(SymbolId("2".into()), EdgeDirection::Outgoing)
            .is_empty());
    }

    #[test]
    fn filtered_relationships_respect_kind_and_confidence() {
        let mut low = rel(2, "1", "3");
        low.confidence = 0.2;
        let graph = CodeGraph::new(
            vec![sym("1", "a"), sym("2", "b"), sym("3", "c")],
            vec![rel(1, "1", "2"), low],
        )
        .expect("valid");

        let calls = graph.relationships_for_filtered(
            SymbolId("1".into()),
            EdgeDirection::Outgoing,
            &[RelationshipKind::Calls],
            0.5,
        );
        assert_eq!(calls.len(), 1, "low-confidence edge should be filtered");

        let imports = graph.relationships_for_filtered(
            SymbolId("1".into()),
            EdgeDirection::Outgoing,
            &[RelationshipKind::Imports],
            0.5,
        );
        assert!(imports.is_empty());
    }

    #[test]
    fn find_is_case_insensitive_and_matches_file() {
        let graph = CodeGraph::new(
            vec![
                sym("1", "parse_input"),
                sym("2", "render_output"),
                sym("3", "parse_config"),
            ],
            Vec::new(),
        )
        .expect("valid graph");

        let hits: Vec<String> = graph.find("parse").iter().map(|s| s.name.clone()).collect();
        assert_eq!(
            hits,
            vec!["parse_input".to_string(), "parse_config".to_string()]
        );
        assert!(graph.find("zzz_nope").is_empty());
    }

    #[test]
    fn duplicate_symbol_id_is_rejected() {
        let result = CodeGraph::new(vec![sym("1", "a"), sym("1", "b")], Vec::new());
        assert!(matches!(
            result,
            Err(CodegraphError::DuplicateSymbolId { id }) if id == SymbolId("1".into())
        ));
    }

    #[test]
    fn files_map_is_exposed() {
        let mut files = HashMap::new();
        files.insert(
            "src/lib.rs".to_string(),
            FileInfo {
                path: "src/lib.rs".into(),
                lang: Some("rust".into()),
                loc: 10,
                symbol_count: 2,
                status: FileStatus::Indexed,
                complexity_sum: 0,
                complexity_max: 0,
                function_count: 0,
                ignores: Vec::new(),
            },
        );
        let graph =
            CodeGraph::new_with_files(vec![sym("1", "a")], Vec::new(), files).expect("valid");
        assert_eq!(graph.file("src/lib.rs").map(|f| f.loc), Some(10));
    }

    #[test]
    fn default_is_empty() {
        let graph = CodeGraph::default();
        assert_eq!(graph.symbol_count(), 0);
        assert!(graph.find("anything").is_empty());
    }
}
