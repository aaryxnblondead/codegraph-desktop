//! Rust extractor using tree-sitter.
//!
//! Extracts functions, structs, enums, traits, impls, modules, consts, statics
//! and type aliases; `use` paths as imports; calls and trait impls as refs.

use std::path::Path;

use tree_sitter::{Node, Parser};

use super::super::facts::{FileFacts, FunctionMetric, RawImport, RawRef, RawSymbol, RefKind};
use super::LanguageExtractor;

pub struct RustExtractor;

impl LanguageExtractor for RustExtractor {
    fn language_id(&self) -> &'static str {
        "rust"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }

    fn extract(&self, _path: &Path, src: &str) -> FileFacts {
        let mut facts = FileFacts::default();
        let language = tree_sitter::Language::new(tree_sitter_rust::LANGUAGE);
        let mut parser = Parser::new();
        if parser.set_language(&language).is_err() {
            return facts;
        }
        let Some(tree) = parser.parse(src, None) else {
            return facts;
        };
        facts.metrics.loc = src.lines().count();
        let mut ctx = Ctx {
            src,
            facts: &mut facts,
            prefix: Vec::new(),
        };
        ctx.walk(tree.root_node(), false);
        facts
    }
}

struct Ctx<'a> {
    src: &'a str,
    facts: &'a mut FileFacts,
    /// Enclosing scope names (`mod` names, `impl` type names) for `Type::method`.
    prefix: Vec<String>,
}

impl Ctx<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        node.utf8_text(self.src.as_bytes()).unwrap_or("")
    }

    fn line(&self, node: Node<'_>) -> u32 {
        node.start_position().row as u32 + 1
    }

    fn is_pub(&self, node: Node<'_>) -> bool {
        let mut cursor = node.walk();
        let has = node
            .named_children(&mut cursor)
            .any(|c| c.kind() == "visibility_modifier");
        has
    }

    fn add_symbol(&mut self, kind: &str, name: &str, node: Node<'_>, exported: bool) -> usize {
        let qualified = if self.prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}::{name}", self.prefix.join("::"))
        };
        let idx = self.facts.symbols.len();
        self.facts.symbols.push(RawSymbol {
            kind: kind.to_string(),
            name: name.to_string(),
            qualified_name: qualified,
            start_line: self.line(node),
            end_line: node.end_position().row as u32 + 1,
            exported,
            parent: None,
        });
        idx
    }

    fn walk(&mut self, node: Node<'_>, exported: bool) {
        match node.kind() {
            "function_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    let pub_ = self.is_pub(node);
                    let kind = if self.prefix.is_empty() {
                        "function"
                    } else {
                        "method"
                    };
                    self.add_symbol(kind, &name, node, pub_ || exported);
                    self.record_metric(node, &name);
                    self.prefix.push(name);
                    if let Some(body) = node.child_by_field_name("body") {
                        self.walk_children(body);
                    }
                    self.prefix.pop();
                }
                return;
            }
            "struct_item" | "enum_item" | "trait_item" | "type_item" | "mod_item" => {
                let (kind, field) = match node.kind() {
                    "struct_item" => ("struct", "name"),
                    "enum_item" => ("enum", "name"),
                    "trait_item" => ("trait", "name"),
                    "type_item" => ("type", "name"),
                    _ => ("mod", "name"),
                };
                if let Some(name_node) = node.child_by_field_name(field) {
                    let name = self.text(name_node).to_string();
                    if kind == "mod" {
                        // `mod foo;` has no body; only inline modules scope symbols.
                        self.add_symbol(kind, &name, node, self.is_pub(node) || exported);
                        if let Some(body) = node.child_by_field_name("body") {
                            self.prefix.push(name);
                            self.walk_children(body);
                            self.prefix.pop();
                        }
                        return;
                    }
                    self.add_symbol(kind, &name, node, self.is_pub(node) || exported);
                }
                return;
            }
            "const_item" | "static_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    let kind = if node.kind() == "const_item" {
                        "constant"
                    } else {
                        "variable"
                    };
                    self.add_symbol(kind, &name, node, self.is_pub(node) || exported);
                }
                return;
            }
            "impl_item" => {
                let trait_name = node
                    .child_by_field_name("trait")
                    .map(|n| self.text(n).to_string());
                let type_name = node
                    .child_by_field_name("type")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let line = self.line(node);
                if let Some(t) = &trait_name {
                    let short = t.rsplit("::").next().unwrap_or(t).to_string();
                    self.facts.refs.push(RawRef {
                        kind: RefKind::Implements,
                        name: short,
                        enclosing: None,
                        line,
                    });
                    let _ = type_name;
                }
                if let Some(body) = node.child_by_field_name("body") {
                    let scope = type_name
                        .split('<')
                        .next()
                        .unwrap_or(&type_name)
                        .rsplit("::")
                        .next()
                        .unwrap_or(&type_name)
                        .trim()
                        .to_string();
                    if !scope.is_empty() {
                        self.prefix.push(scope.clone());
                    }
                    self.walk_children(body);
                    if !scope.is_empty() {
                        self.prefix.pop();
                    }
                }
                return;
            }
            "use_declaration" => {
                self.handle_use(node);
                return;
            }
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    // Method calls: keep the method name (`x.foo()` -> `foo`) so
                    // the method can still be matched by name.
                    let name = if func.kind() == "field_expression" {
                        func.child_by_field_name("field")
                            .map(|f| self.text(f).to_string())
                            .unwrap_or_default()
                    } else {
                        self.text(func).to_string()
                    };
                    if !name.is_empty() {
                        self.facts.refs.push(RawRef {
                            kind: RefKind::Call,
                            name,
                            enclosing: None,
                            line: self.line(node),
                        });
                    }
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, exported);
        }
    }

    fn walk_children(&mut self, node: Node<'_>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, false);
        }
    }

    /// Flatten a `use` tree into `(full_path, item_name, local_name)` triples.
    fn handle_use(&mut self, node: Node<'_>) {
        let Some(arg) = node.child_by_field_name("argument") else {
            return;
        };
        let line = self.line(node);
        let mut out = Vec::new();
        flatten_use(self, arg, "", &mut out);
        for (path, local) in out {
            let item = path.rsplit("::").next().unwrap_or(&path).to_string();
            self.facts.imports.push(RawImport {
                specifier: path,
                names: vec![item],
                local_names: vec![local],
                namespace: None,
                default: None,
                is_reexport: false,
                line,
            });
        }
    }

    fn record_metric(&mut self, node: Node<'_>, name: &str) {
        let params = node
            .child_by_field_name("parameters")
            .map(|p| {
                let mut cursor = p.walk();
                p.named_children(&mut cursor)
                    .filter(|c| c.kind() != "comment")
                    .count() as u32
            })
            .unwrap_or(0);
        let body = node.child_by_field_name("body").unwrap_or(node);
        let (cyclomatic, nesting) = scan_complexity(body);
        let loc = node
            .end_position()
            .row
            .saturating_sub(node.start_position().row)
            + 1;
        self.facts.metrics.functions.push(FunctionMetric {
            name: name.to_string(),
            line: self.line(node),
            end_line: node.end_position().row as u32 + 1,
            cyclomatic,
            nesting,
            params,
            loc,
            render: false,
        });
    }
}

/// Walk a `use` argument, accumulating scoped paths.
fn flatten_use(ctx: &Ctx<'_>, node: Node<'_>, prefix: &str, out: &mut Vec<(String, String)>) {
    let join = |a: &str, b: &str| {
        if a.is_empty() {
            b.to_string()
        } else if b.is_empty() {
            a.to_string()
        } else {
            format!("{a}::{b}")
        }
    };
    match node.kind() {
        "scoped_identifier" => {
            let path = join(prefix, ctx.text(node).trim());
            let local = path.rsplit("::").next().unwrap_or(&path).to_string();
            out.push((path, local));
        }
        "identifier" | "crate" | "self" | "super" | "metavariable" => {
            let path = join(prefix, ctx.text(node).trim());
            let local = path.rsplit("::").next().unwrap_or(&path).to_string();
            out.push((path, local));
        }
        "use_as_clause" => {
            let path = node
                .child_by_field_name("path")
                .map(|n| join(prefix, ctx.text(n).trim()))
                .unwrap_or_default();
            let alias = node
                .child_by_field_name("alias")
                .map(|n| ctx.text(n).trim().to_string())
                .unwrap_or_else(|| path.rsplit("::").next().unwrap_or("").to_string());
            if !path.is_empty() {
                out.push((path, alias));
            }
        }
        "scoped_use_list" => {
            let head = node
                .child_by_field_name("path")
                .map(|n| join(prefix, ctx.text(n).trim()))
                .unwrap_or_else(|| prefix.to_string());
            if let Some(list) = node.child_by_field_name("list") {
                let mut cursor = list.walk();
                for child in list.named_children(&mut cursor) {
                    flatten_use(ctx, child, &head, out);
                }
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                flatten_use(ctx, child, prefix, out);
            }
        }
        "use_wildcard" => {
            // `use foo::*` — keep the module path so it still creates an edge.
            if let Some(n) = node.named_child(0) {
                let path = join(prefix, ctx.text(n).trim());
                out.push((path.clone(), path));
            }
        }
        _ => {
            let text = ctx.text(node).trim();
            if !text.is_empty() {
                let path = join(prefix, text);
                let local = path.rsplit("::").next().unwrap_or(&path).to_string();
                out.push((path, local));
            }
        }
    }
}

fn scan_complexity(node: Node<'_>) -> (u32, u32) {
    let mut cyclomatic = 1u32;
    let mut max_depth = 0u32;
    let mut stack = vec![(node, 0u32)];
    while let Some((n, depth)) = stack.pop() {
        let is_decision = match n.kind() {
            "if_expression" | "while_expression" | "for_expression" | "match_arm" => true,
            "binary_expression" => matches!(
                n.child_by_field_name("operator").map(|o| o.kind()),
                Some("&&") | Some("||")
            ),
            _ => false,
        };
        if is_decision {
            cyclomatic += 1;
            max_depth = max_depth.max(depth + 1);
        }
        let next = if is_decision { depth + 1 } else { depth };
        let mut cursor = n.walk();
        for child in n.named_children(&mut cursor) {
            stack.push((child, next));
        }
    }
    (cyclomatic, max_depth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn extract(src: &str) -> FileFacts {
        RustExtractor.extract(&PathBuf::from("src/a.rs"), src)
    }

    #[test]
    fn extracts_items_and_impl_methods() {
        let facts = extract(
            r#"
            pub struct Foo { x: u32 }
            pub enum E { A, B }
            pub trait T { fn go(&self); }
            impl T for Foo { fn go(&self) {} }
            impl Foo { pub fn new() -> Self { Foo { x: 0 } } }
            pub const LIMIT: u32 = 3;
            static NAME: &str = "a";
            pub type Alias = u32;
            mod inner { pub fn helper() {} }
            fn plain() {}
            "#,
        );
        let names: Vec<&str> = facts.symbols.iter().map(|s| s.name.as_str()).collect();
        for expected in [
            "Foo", "E", "T", "go", "new", "LIMIT", "NAME", "Alias", "inner", "helper", "plain",
        ] {
            assert!(names.contains(&expected), "missing {expected} in {names:?}");
        }
        assert!(facts
            .symbols
            .iter()
            .any(|s| s.qualified_name == "Foo::new" && s.kind == "method"));
        assert!(facts
            .symbols
            .iter()
            .any(|s| s.kind == "struct" && s.exported));
    }

    #[test]
    fn extracts_use_paths_and_impl_trait() {
        let facts = extract(
            r#"
            use crate::metrics::compute;
            use std::collections::HashMap;
            use super::graph::CodeGraph as Graph;
            use crate::modules::{A, B};
            use crate::*;
            "#,
        );
        let specifiers: Vec<&str> = facts.imports.iter().map(|i| i.specifier.as_str()).collect();
        assert!(
            specifiers.contains(&"crate::metrics::compute"),
            "{specifiers:?}"
        );
        assert!(
            specifiers.contains(&"std::collections::HashMap"),
            "{specifiers:?}"
        );
        assert!(
            specifiers
                .iter()
                .any(|s| s.ends_with("super::graph::CodeGraph")),
            "{specifiers:?}"
        );
        assert!(facts
            .imports
            .iter()
            .any(|i| i.specifier == "crate::modules::A" && i.local_names == vec!["A".to_string()]));
        assert!(specifiers.contains(&"crate"), "{specifiers:?}");
    }

    #[test]
    fn impl_trait_creates_implements_ref() {
        let facts = extract("struct S; trait T {} impl T for S {}");
        assert!(facts
            .refs
            .iter()
            .any(|r| r.kind == RefKind::Implements && r.name == "T"));
    }

    #[test]
    fn records_rust_complexity() {
        let facts = extract(
            r#"
            fn f(a: u32, b: u32) -> u32 {
                if a > 0 && b > 0 {
                    for i in 0..3 { let _ = i; }
                }
                match a {
                    0 => 1,
                    _ => 2,
                }
            }
            "#,
        );
        let m = facts
            .metrics
            .functions
            .iter()
            .find(|m| m.name == "f")
            .unwrap();
        // 1 + if + && + for + 2 match arms = 6
        assert_eq!(m.cyclomatic, 6, "got {}", m.cyclomatic);
        assert_eq!(m.params, 2);
        assert!(m.nesting >= 2);
    }

    #[test]
    fn never_panics_on_garbage() {
        let _ = extract("fn ( { [ impl for");
        let _ = extract("\u{0}\u{1}\u{2}");
        let _ = extract("");
    }
}
