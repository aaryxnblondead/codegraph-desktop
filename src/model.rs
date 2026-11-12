//! Domain model for a CodeGraph knowledge graph.
//!
//! A graph is made of `Symbol`s (nodes) and `Relationship`s (edges). Every
//! identifier is a branded `SymbolId` so raw integers can't leak into symbol
//! slots. Kinds are extensible: unknown strings from a legacy database map to
//! `Other(String)` rather than failing.

use std::fmt;

/// A branded, opaque symbol identifier.
///
/// CodeGraph stores node ids as TEXT (UUIDs/hashes); the indexer uses stable
/// content-addressed ids. Wrapping `String` prevents passing a raw file path
/// or relationship id where a symbol id is expected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(pub String);

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for SymbolId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl AsRef<str> for SymbolId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// The kind of symbol (node) in the graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum SymbolKind {
    #[default]
    Function,
    Method,
    Class,
    Interface,
    Trait,
    Struct,
    Enum,
    TypeAlias,
    Module,
    Namespace,
    Constant,
    Variable,
    /// Unknown/legacy kind, preserved verbatim.
    Other(String),
}

impl SymbolKind {
    /// Parse a kind string, mapping unknown values to `Other`.
    pub fn parse(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "function" | "fn" => SymbolKind::Function,
            "method" => SymbolKind::Method,
            "class" => SymbolKind::Class,
            "interface" => SymbolKind::Interface,
            "trait" => SymbolKind::Trait,
            "struct" => SymbolKind::Struct,
            "enum" => SymbolKind::Enum,
            "typealias" | "type_alias" | "type" => SymbolKind::TypeAlias,
            "module" | "mod" => SymbolKind::Module,
            "namespace" => SymbolKind::Namespace,
            "constant" | "const" => SymbolKind::Constant,
            "variable" | "var" => SymbolKind::Variable,
            _ => SymbolKind::Other(value.to_string()),
        }
    }

    /// Short display label.
    pub fn short(&self) -> &str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Trait => "trait",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Module => "mod",
            SymbolKind::Namespace => "ns",
            SymbolKind::Constant => "const",
            SymbolKind::Variable => "var",
            SymbolKind::Other(s) => s.as_str(),
        }
    }

    /// Whether this is a type-like symbol (for abstractness).
    pub fn is_type_like(&self) -> bool {
        matches!(
            self,
            SymbolKind::Class
                | SymbolKind::Interface
                | SymbolKind::Trait
                | SymbolKind::Struct
                | SymbolKind::Enum
        )
    }

    /// Whether this is an abstract type (for abstractness).
    pub fn is_abstract_type(&self) -> bool {
        matches!(self, SymbolKind::Interface | SymbolKind::Trait)
    }
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.short())
    }
}

impl From<&str> for SymbolKind {
    fn from(value: &str) -> Self {
        SymbolKind::parse(value)
    }
}

/// A single node in the code graph.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub file: String,
    /// 1-based start line.
    pub line: Option<u32>,
    /// 1-based end line.
    pub end_line: Option<u32>,
    pub signature: Option<String>,
    pub doc: Option<String>,
    /// Dotted/`::` qualified name, e.g. `Type::method`.
    pub qualified_name: String,
    /// Whether the symbol is exported/public.
    pub exported: bool,
    /// Source language id (`typescript`, `python`, `rust`, ...).
    pub lang: Option<String>,
    /// Cyclomatic complexity for functions/methods.
    pub complexity: Option<u32>,
    /// True for JS/TS functions that render JSX (React components).
    pub is_render: bool,
}

impl Symbol {
    /// Construct with the common fields; the rest default.
    pub fn new(
        id: SymbolId,
        name: impl Into<String>,
        kind: SymbolKind,
        file: impl Into<String>,
    ) -> Self {
        let name = name.into();
        Self {
            qualified_name: name.clone(),
            id,
            name,
            kind,
            file: file.into(),
            line: None,
            end_line: None,
            signature: None,
            doc: None,
            exported: false,
            lang: None,
            complexity: None,
            is_render: false,
        }
    }

    /// The short label shown in lists and headers.
    pub fn label(&self) -> String {
        match &self.signature {
            Some(sig) if !sig.is_empty() => format!("{} {}", self.kind, sig),
            _ => format!("{} {}", self.kind, self.name),
        }
    }
}

/// The kind of relationship (edge) between two symbols.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum RelationshipKind {
    #[default]
    Calls,
    Defines,
    References,
    /// Type reference (annotation, parameter type, etc.).
    TypeRef,
    /// Base class / superclass.
    Extends,
    /// Alias kept for legacy DBs; treated like `Extends`.
    Inherits,
    Implements,
    Imports,
    Contains,
    /// Unknown/legacy kind, preserved verbatim.
    Other(String),
}

impl RelationshipKind {
    pub fn parse(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "calls" | "call" => RelationshipKind::Calls,
            "defines" | "define" => RelationshipKind::Defines,
            "references" | "reference" | "ref" => RelationshipKind::References,
            "type_ref" | "typeref" | "type" => RelationshipKind::TypeRef,
            "extends" => RelationshipKind::Extends,
            "inherits" | "inheritance" => RelationshipKind::Inherits,
            "implements" | "implementation" => RelationshipKind::Implements,
            "imports" | "import" => RelationshipKind::Imports,
            "contains" | "contain" => RelationshipKind::Contains,
            _ => RelationshipKind::Other(value.to_string()),
        }
    }

    pub fn short(&self) -> &str {
        match self {
            RelationshipKind::Calls => "calls",
            RelationshipKind::Defines => "defines",
            RelationshipKind::References => "references",
            RelationshipKind::TypeRef => "type_ref",
            RelationshipKind::Extends => "extends",
            RelationshipKind::Inherits => "inherits",
            RelationshipKind::Implements => "implements",
            RelationshipKind::Imports => "imports",
            RelationshipKind::Contains => "contains",
            RelationshipKind::Other(s) => s.as_str(),
        }
    }

    /// Structural edges (containment) are excluded from coupling metrics.
    pub fn is_structural(&self) -> bool {
        matches!(self, RelationshipKind::Contains)
    }
}

impl fmt::Display for RelationshipKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.short())
    }
}

impl From<&str> for RelationshipKind {
    fn from(value: &str) -> Self {
        RelationshipKind::parse(value)
    }
}

/// A single directed edge: `source` --kind--> `target`.
#[derive(Debug, Clone)]
pub struct Relationship {
    pub id: u64,
    pub source_id: SymbolId,
    pub target_id: SymbolId,
    pub kind: RelationshipKind,
    /// Source line of the reference, if known.
    pub line: Option<u32>,
    /// Resolution confidence in 0..=1 (1.0 for legacy rows).
    pub confidence: f32,
    /// Where the edge came from (`legacy`, `import`, `local`, ...).
    pub provenance: String,
}

impl Relationship {
    /// Construct a legacy/structural edge with maximum confidence.
    pub fn new(id: u64, source_id: SymbolId, target_id: SymbolId, kind: RelationshipKind) -> Self {
        Self {
            id,
            source_id,
            target_id,
            kind,
            line: None,
            confidence: 1.0,
            provenance: "legacy".to_string(),
        }
    }

    /// Construct an indexed edge with provenance + confidence.
    pub fn indexed(
        id: u64,
        source_id: SymbolId,
        target_id: SymbolId,
        kind: RelationshipKind,
        line: Option<u32>,
        confidence: f32,
        provenance: impl Into<String>,
    ) -> Self {
        Self {
            id,
            source_id,
            target_id,
            kind,
            line,
            confidence,
            provenance: provenance.into(),
        }
    }
}

/// Per-file information kept alongside the graph.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub path: String,
    pub lang: Option<String>,
    pub loc: usize,
    pub symbol_count: usize,
    pub status: FileStatus,
    /// Sum of per-function cyclomatic complexity.
    pub complexity_sum: u32,
    /// Maximum per-function cyclomatic complexity.
    pub complexity_max: u32,
    /// Number of functions/methods.
    pub function_count: usize,
    /// Inline `codegraph-ignore` comments for this file.
    pub ignores: Vec<crate::indexer::facts::Ignore>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FileStatus {
    #[default]
    Indexed,
    Cached,
    SkippedLarge,
    Failed(String),
}

/// Direction of traversal around a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDirection {
    Outgoing,
    Incoming,
}

impl fmt::Display for EdgeDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EdgeDirection::Outgoing => write!(f, "outgoing"),
            EdgeDirection::Incoming => write!(f, "incoming"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_id_is_opaque() {
        assert_eq!(SymbolId("1".into()), SymbolId("1".into()));
        assert_ne!(SymbolId("1".into()), SymbolId("2".into()));
    }

    #[test]
    fn symbol_label_defaults_to_name() {
        let sym = Symbol::new(
            SymbolId("1".into()),
            "parse_input",
            SymbolKind::Function,
            "src/parser.rs",
        );
        assert_eq!(sym.label(), "fn parse_input");
    }

    #[test]
    fn label_uses_signature_when_present() {
        let mut sym = Symbol::new(
            SymbolId("2".into()),
            "parse_input",
            SymbolKind::Function,
            "src/parser.rs",
        );
        sym.signature = Some("(s: &str) -> Result<Vec<Token>, ParseError>".into());
        assert_eq!(
            sym.label(),
            "fn (s: &str) -> Result<Vec<Token>, ParseError>"
        );
    }

    #[test]
    fn unknown_kinds_are_preserved() {
        assert_eq!(
            SymbolKind::parse("weird-kind"),
            SymbolKind::Other("weird-kind".into())
        );
        assert_eq!(
            RelationshipKind::parse("weird-edge"),
            RelationshipKind::Other("weird-edge".into())
        );
        assert_eq!(SymbolKind::parse("METHOD"), SymbolKind::Method);
    }

    #[test]
    fn kind_display_and_traits() {
        assert_eq!(RelationshipKind::Calls.to_string(), "calls");
        assert_eq!(SymbolKind::Interface.to_string(), "interface");
        assert!(SymbolKind::Trait.is_abstract_type());
        assert!(SymbolKind::Struct.is_type_like());
        assert!(RelationshipKind::Contains.is_structural());
        assert!(!RelationshipKind::Calls.is_structural());
    }

    #[test]
    fn edge_direction_display() {
        assert_eq!(EdgeDirection::Outgoing.to_string(), "outgoing");
        assert_eq!(EdgeDirection::Incoming.to_string(), "incoming");
    }
}
