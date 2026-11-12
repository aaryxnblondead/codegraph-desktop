//! Raw per-file facts produced by a language extractor, before resolution.

use serde::{Deserialize, Serialize};

/// A symbol declared in a file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawSymbol {
    /// `function`, `method`, `class`, `interface`, `trait`, `struct`, `enum`,
    /// `type`, `module`, `constant`, `variable`.
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub exported: bool,
    /// Index of the parent symbol within this file, if nested.
    pub parent: Option<usize>,
}

/// An import (or re-export) statement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawImport {
    /// The specifier as written, e.g. `./foo` or `@/lib/bar`.
    pub specifier: String,
    /// Imported names (empty for a side-effect or namespace import).
    pub names: Vec<String>,
    /// Local binding names, parallel to `names` (alias if renamed).
    pub local_names: Vec<String>,
    /// Namespace import alias (`import * as ns`).
    pub namespace: Option<String>,
    /// Default import alias (`import x from`).
    pub default: Option<String>,
    /// `export ... from` (re-export).
    pub is_reexport: bool,
    pub line: u32,
}

/// A reference to a name (call, type use, extends, implements).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawRef {
    pub kind: RefKind,
    /// The referenced name path, e.g. `foo` or `ns.foo` or `Type::method`.
    pub name: String,
    /// Index of the enclosing symbol in `FileFacts.symbols`, if any.
    pub enclosing: Option<usize>,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RefKind {
    Call,
    Type,
    Extends,
    Implements,
}

/// Per-file metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RawMetrics {
    pub loc: usize,
    pub functions: Vec<FunctionMetric>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionMetric {
    pub name: String,
    /// 1-based line of the function.
    pub line: u32,
    /// 1-based end line of the function.
    #[serde(default)]
    pub end_line: u32,
    pub cyclomatic: u32,
    pub nesting: u32,
    pub params: u32,
    pub loc: usize,
    /// True when the body renders JSX (a React component / render function).
    #[serde(default)]
    pub render: bool,
}

/// An inline suppression comment: `codegraph-ignore <rule>[, <rule>…]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ignore {
    /// 1-based line the comment sits on.
    pub line: u32,
    pub rules: Vec<String>,
}

/// Scan source for `codegraph-ignore` comments.
///
/// Works for `//`, `#` and `/* … */` comment styles because it only looks at
/// the marker. A comment without rule names suppresses every rule on that line.
pub fn parse_ignores(src: &str) -> Vec<Ignore> {
    let mut out = Vec::new();
    for (idx, line) in src.lines().enumerate() {
        let Some(pos) = line.find("codegraph-ignore") else {
            continue;
        };
        let rest = &line[pos + "codegraph-ignore".len()..];
        let rest = rest.split("*/").next().unwrap_or(rest);
        let rules: Vec<String> = rest
            .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
            .filter(|w| !w.is_empty())
            .map(|w| w.to_lowercase())
            .collect();
        out.push(Ignore {
            line: idx as u32 + 1,
            rules,
        });
    }
    out
}

/// Everything extracted from one file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileFacts {
    pub symbols: Vec<RawSymbol>,
    pub imports: Vec<RawImport>,
    pub refs: Vec<RawRef>,
    pub metrics: RawMetrics,
    /// Inline `codegraph-ignore` comments.
    #[serde(default)]
    pub ignores: Vec<Ignore>,
}

impl FileFacts {
    /// Find a symbol index by name (first match).
    pub fn symbol_index(&self, name: &str) -> Option<usize> {
        self.symbols.iter().position(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_round_trip() {
        let facts = FileFacts {
            symbols: vec![RawSymbol {
                kind: "function".into(),
                name: "f".into(),
                qualified_name: "f".into(),
                start_line: 1,
                end_line: 2,
                exported: true,
                parent: None,
            }],
            imports: vec![RawImport {
                specifier: "./x".into(),
                names: vec!["a".into()],
                local_names: vec!["a".into()],
                namespace: None,
                default: None,
                is_reexport: false,
                line: 1,
            }],
            refs: vec![RawRef {
                kind: RefKind::Call,
                name: "a".into(),
                enclosing: Some(0),
                line: 2,
            }],
            metrics: RawMetrics::default(),
            ignores: Vec::new(),
        };
        let json = serde_json::to_string(&facts).unwrap();
        let back: FileFacts = serde_json::from_str(&json).unwrap();
        assert_eq!(back.symbols[0].name, "f");
        assert_eq!(back.imports[0].specifier, "./x");
    }

    #[test]
    fn symbol_index_finds_first() {
        let facts = FileFacts {
            symbols: vec![
                RawSymbol {
                    kind: "function".into(),
                    name: "a".into(),
                    qualified_name: "a".into(),
                    start_line: 1,
                    end_line: 1,
                    exported: false,
                    parent: None,
                },
                RawSymbol {
                    kind: "function".into(),
                    name: "b".into(),
                    qualified_name: "b".into(),
                    start_line: 2,
                    end_line: 2,
                    exported: false,
                    parent: None,
                },
            ],
            ..Default::default()
        };
        assert_eq!(facts.symbol_index("b"), Some(1));
        assert_eq!(facts.symbol_index("z"), None);
    }
}

#[cfg(test)]
mod ignore_tests {
    use super::*;

    #[test]
    fn parses_inline_ignores() {
        let src = "// codegraph-ignore complex_function\nfn f() {}\n# codegraph-ignore low_cohesion, orphan\n/* codegraph-ignore god_file */\n";
        let ignores = parse_ignores(src);
        assert_eq!(ignores.len(), 3);
        assert_eq!(ignores[0].line, 1);
        assert_eq!(ignores[0].rules, vec!["complex_function"]);
        assert_eq!(ignores[1].rules, vec!["low_cohesion", "orphan"]);
        assert_eq!(ignores[2].rules, vec!["god_file"]);
    }

    #[test]
    fn bare_ignore_means_every_rule() {
        let ignores = parse_ignores("// codegraph-ignore\n");
        assert_eq!(ignores.len(), 1);
        assert!(ignores[0].rules.is_empty());
    }
}
