//! TypeScript / JavaScript / TSX extractor using tree-sitter.

use std::path::Path;

use tree_sitter::{Node, Parser};

use super::super::facts::{FileFacts, FunctionMetric, RawImport, RawRef, RawSymbol, RefKind};
use super::LanguageExtractor;

pub struct TypeScriptExtractor;

const EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];

impl LanguageExtractor for TypeScriptExtractor {
    fn language_id(&self) -> &'static str {
        "typescript"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn extract(&self, path: &Path, src: &str) -> FileFacts {
        let mut facts = FileFacts::default();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let language = if ext == "ts" {
            tree_sitter::Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT)
        } else {
            tree_sitter::Language::new(tree_sitter_typescript::LANGUAGE_TSX)
        };
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
            stack: Vec::new(),
        };
        ctx.walk(tree.root_node(), None, "", false);
        facts
    }
}

struct Ctx<'a> {
    src: &'a str,
    facts: &'a mut FileFacts,
    stack: Vec<usize>,
}

impl Ctx<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        node.utf8_text(self.src.as_bytes()).unwrap_or("")
    }

    fn line(&self, node: Node<'_>) -> u32 {
        node.start_position().row as u32 + 1
    }

    fn enclosing(&self) -> Option<usize> {
        self.stack.last().copied()
    }

    fn add_symbol(
        &mut self,
        kind: &str,
        name: &str,
        node: Node<'_>,
        prefix: &str,
        exported: bool,
    ) -> usize {
        let qualified = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}::{name}")
        };
        let idx = self.facts.symbols.len();
        self.facts.symbols.push(RawSymbol {
            kind: kind.to_string(),
            name: name.to_string(),
            qualified_name: qualified,
            start_line: self.line(node),
            end_line: node.end_position().row as u32 + 1,
            exported,
            parent: self.enclosing(),
        });
        idx
    }

    fn walk(&mut self, node: Node<'_>, _parent: Option<usize>, prefix: &str, exported: bool) {
        let kind = node.kind();
        match kind {
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    let idx = self.add_symbol("function", &name, node, prefix, exported);
                    self.record_function_metric(node, &name);
                    self.push_and_walk(idx, node, &name, "body");
                }
                return;
            }
            "class_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    let idx = self.add_symbol("class", &name, node, prefix, exported);
                    self.collect_heritage(node, idx);
                    self.push_and_walk(idx, node, &name, "body");
                }
                return;
            }
            "interface_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    let idx = self.add_symbol("interface", &name, node, prefix, exported);
                    self.collect_heritage(node, idx);
                    self.push_and_walk(idx, node, &name, "body");
                }
                return;
            }
            "type_alias_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    let idx = self.add_symbol("type", &name, node, prefix, exported);
                    self.collect_type_refs(node, idx);
                    let _ = idx;
                }
                return;
            }
            "enum_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    self.add_symbol("enum", &name, node, prefix, exported);
                }
                return;
            }
            "method_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    let idx = self.add_symbol("method", &name, node, prefix, exported);
                    self.record_function_metric(node, &name);
                    self.push_and_walk(idx, node, &name, "body");
                }
                return;
            }
            "lexical_declaration" | "variable_declaration" => {
                self.walk_variables(node, prefix, exported);
                return;
            }
            "import_statement" => {
                self.handle_import(node, false);
                return;
            }
            "export_statement" => {
                self.handle_export(node, prefix);
                return;
            }
            "call_expression" => {
                if let Some(func) = node.child_by_field_name("function") {
                    let name = self.text(func).to_string();
                    if is_name_like(func.kind()) && !name.is_empty() {
                        let line = self.line(node);
                        let enclosing = self.enclosing();
                        self.facts.refs.push(RawRef {
                            kind: RefKind::Call,
                            name,
                            enclosing,
                            line,
                        });
                    }
                }
            }
            "new_expression" => {
                if let Some(ctor) = node.child_by_field_name("constructor") {
                    let name = self.text(ctor).to_string();
                    if !name.is_empty() {
                        let line = self.line(node);
                        let enclosing = self.enclosing();
                        self.facts.refs.push(RawRef {
                            kind: RefKind::Type,
                            name,
                            enclosing,
                            line,
                        });
                    }
                }
            }
            "extends_clause" => {
                self.collect_clause(node, RefKind::Extends);
            }
            "implements_clause" => {
                self.collect_clause(node, RefKind::Implements);
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, _parent, prefix, exported);
        }
    }

    fn push_and_walk(&mut self, idx: usize, node: Node<'_>, prefix: &str, body_field: &str) {
        self.stack.push(idx);
        if let Some(body) = node.child_by_field_name(body_field) {
            self.walk(body, Some(idx), prefix, false);
        } else {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                self.walk(child, Some(idx), prefix, false);
            }
        }
        self.stack.pop();
    }

    fn walk_variables(&mut self, node: Node<'_>, prefix: &str, exported: bool) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() != "variable_declarator" {
                continue;
            }
            let Some(name_node) = child.child_by_field_name("name") else {
                continue;
            };
            let name = self.text(name_node).to_string();
            let value = child.child_by_field_name("value");
            let is_fn =
                value.is_some_and(|v| matches!(v.kind(), "arrow_function" | "function_expression"));
            let kind = if is_fn { "function" } else { "variable" };
            let idx = self.add_symbol(kind, &name, child, prefix, exported);
            if is_fn {
                self.record_function_metric(child, &name);
            }
            if let Some(v) = value {
                self.stack.push(idx);
                self.walk(v, Some(idx), prefix, false);
                self.stack.pop();
            }
        }
    }

    fn collect_heritage(&mut self, node: Node<'_>, owner: usize) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "extends_clause" => self.collect_clause(child, RefKind::Extends),
                "implements_clause" => self.collect_clause(child, RefKind::Implements),
                "class_heritage" => {
                    let mut c2 = child.walk();
                    for gc in child.named_children(&mut c2) {
                        match gc.kind() {
                            "extends_clause" => self.collect_clause(gc, RefKind::Extends),
                            "implements_clause" => self.collect_clause(gc, RefKind::Implements),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        let _ = owner;
    }

    fn collect_clause(&mut self, node: Node<'_>, kind: RefKind) {
        let enclosing = self.enclosing();
        let line = self.line(node);
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "type_identifier" {
                let name = self.text(child).to_string();
                if !name.is_empty() {
                    self.facts.refs.push(RawRef {
                        kind,
                        name,
                        enclosing,
                        line,
                    });
                }
            }
        }
    }

    fn collect_type_refs(&mut self, node: Node<'_>, owner: usize) {
        let enclosing = self.enclosing();
        let line = self.line(node);
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if n.kind() == "type_identifier" {
                let name = self.text(n).to_string();
                if !name.is_empty() {
                    self.facts.refs.push(RawRef {
                        kind: RefKind::Type,
                        name,
                        enclosing,
                        line,
                    });
                }
            }
            let mut cursor = n.walk();
            for child in n.named_children(&mut cursor) {
                stack.push(child);
            }
        }
        let _ = owner;
    }

    fn handle_import(&mut self, node: Node<'_>, is_reexport: bool) {
        let specifier = node
            .child_by_field_name("source")
            .map(|n| strip_quotes(self.text(n)))
            .unwrap_or_default();
        if specifier.is_empty() {
            return;
        }
        let mut names = Vec::new();
        let mut local_names = Vec::new();
        let mut namespace = None;
        let mut default = None;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "import_clause" => {
                    let mut c2 = child.walk();
                    for part in child.named_children(&mut c2) {
                        match part.kind() {
                            "identifier" => default = Some(self.text(part).to_string()),
                            "namespace_import" => {
                                if let Some(id) = last_identifier(part) {
                                    namespace = Some(self.text(id).to_string());
                                }
                            }
                            "named_imports" => {
                                let mut c3 = part.walk();
                                for spec in part.named_children(&mut c3) {
                                    if spec.kind() == "import_specifier" {
                                        if let Some(name) = spec.child_by_field_name("name") {
                                            let imported = self.text(name).to_string();
                                            let local = spec
                                                .child_by_field_name("alias")
                                                .map(|a| self.text(a).to_string())
                                                .unwrap_or_else(|| imported.clone());
                                            names.push(imported);
                                            local_names.push(local);
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "export_clause" => {
                    let mut c2 = child.walk();
                    for spec in child.named_children(&mut c2) {
                        if spec.kind() == "export_specifier" {
                            if let Some(name) = spec.child_by_field_name("name") {
                                let n = self.text(name).to_string();
                                names.push(n.clone());
                                local_names.push(n);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        self.facts.imports.push(RawImport {
            specifier,
            names,
            local_names,
            namespace,
            default,
            is_reexport,
            line: self.line(node),
        });
    }

    fn handle_export(&mut self, node: Node<'_>, prefix: &str) {
        let source = node.child_by_field_name("source");
        if source.is_some() {
            self.handle_import(node, true);
            return;
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "export_clause" => continue,
                _ => self.walk(child, None, prefix, true),
            }
        }
    }

    fn record_function_metric(&mut self, node: Node<'_>, name: &str) {
        let params = node
            .child_by_field_name("parameters")
            .or_else(|| node.child_by_field_name("parameter"))
            .map(|p| p.named_child_count() as u32)
            .unwrap_or(0);
        let body = node.child_by_field_name("body").unwrap_or(node);
        let render = body_has_jsx(body);
        let (cyclomatic, nesting) = scan_complexity(body);
        let loc = (node
            .end_position()
            .row
            .saturating_sub(node.start_position().row))
            + 1;
        self.facts.metrics.functions.push(FunctionMetric {
            name: name.to_string(),
            line: self.line(node),
            end_line: node.end_position().row as u32 + 1,
            cyclomatic,
            nesting,
            params,
            loc,
            render,
        });
    }
}

fn is_name_like(kind: &str) -> bool {
    matches!(
        kind,
        "identifier" | "member_expression" | "property_identifier"
    )
}

fn strip_quotes(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

fn last_identifier(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|c| c.kind() == "identifier")
        .last()
}

/// Branches inside JSX markup are presentation, not control flow: a list of
/// `{cond && <Row/>}` in a render function would otherwise look like 50 paths.
/// They are excluded from cyclomatic complexity when the function renders JSX.
fn scan_complexity(node: Node<'_>) -> (u32, u32) {
    let mut cyclomatic = 1u32;
    let mut max_depth = 0u32;
    let mut stack = vec![(node, 0u32, false)];
    while let Some((n, depth, in_jsx)) = stack.pop() {
        let jsx = in_jsx || n.kind().starts_with("jsx_");
        let is_decision = !jsx
            && matches!(
                n.kind(),
                "if_statement"
                    | "for_statement"
                    | "for_in_statement"
                    | "while_statement"
                    | "do_statement"
                    | "switch_case"
                    | "catch_clause"
                    | "ternary_expression"
            );
        let is_bool = !jsx
            && n.kind() == "binary_expression"
            && matches!(
                n.child_by_field_name("operator").map(|o| o.kind()),
                Some("&&") | Some("||") | Some("??")
            );
        if is_decision || is_bool {
            cyclomatic += 1;
            max_depth = max_depth.max(depth + 1);
        }
        let next_depth = if is_decision { depth + 1 } else { depth };
        let mut cursor = n.walk();
        for child in n.named_children(&mut cursor) {
            stack.push((child, next_depth, jsx));
        }
    }
    (cyclomatic, max_depth)
}

/// Does this function body render JSX?
fn body_has_jsx(node: Node<'_>) -> bool {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "jsx_element"
            || n.kind() == "jsx_self_closing_element"
            || n.kind() == "jsx_fragment"
        {
            return true;
        }
        let mut cursor = n.walk();
        for child in n.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn extract(src: &str) -> FileFacts {
        TypeScriptExtractor.extract(&PathBuf::from("src/a.ts"), src)
    }

    #[test]
    fn extracts_functions_and_classes() {
        let facts = extract(
            r#"
            export function foo(a: number) { return a + 1; }
            class Bar { method() { return 1; } }
            const baz = (x) => x * 2;
            interface IThing { value: string; }
            type Alias = IThing;
            enum Color { Red, Green }
            "#,
        );
        let names: Vec<&str> = facts.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"foo"), "{names:?}");
        assert!(names.contains(&"Bar"), "{names:?}");
        assert!(names.contains(&"method"), "{names:?}");
        assert!(names.contains(&"baz"), "{names:?}");
        assert!(names.contains(&"IThing"), "{names:?}");
        assert!(names.contains(&"Alias"), "{names:?}");
        assert!(names.contains(&"Color"), "{names:?}");
        assert!(facts.symbols.iter().any(|s| s.name == "foo" && s.exported));
    }

    #[test]
    fn extracts_imports_and_reexports() {
        let facts = extract(
            r#"
            import foo, { bar as baz, qux } from "./mod";
            import * as ns from "./ns";
            export { a, b } from "./other";
            export * from "./all";
            "#,
        );
        assert!(facts.imports.iter().any(|i| i.specifier == "./mod"
            && i.names.contains(&"bar".to_string())
            && i.default.as_deref() == Some("foo")));
        assert!(facts
            .imports
            .iter()
            .any(|i| i.specifier == "./ns" && i.namespace.as_deref() == Some("ns")));
        assert!(facts
            .imports
            .iter()
            .any(|i| i.specifier == "./other" && i.is_reexport));
        assert!(facts
            .imports
            .iter()
            .any(|i| i.specifier == "./all" && i.is_reexport));
    }

    #[test]
    fn records_cyclomatic_complexity() {
        let facts = extract(
            r#"
            function f(a) {
                if (a) { return 1; }
                for (let i = 0; i < 3; i++) { if (i) { a++; } }
                return a && 1;
            }
            "#,
        );
        let m = facts
            .metrics
            .functions
            .iter()
            .find(|m| m.name == "f")
            .expect("metric");
        // 1 + if + for + if + && = 5
        assert_eq!(m.cyclomatic, 5, "got {}", m.cyclomatic);
        assert!(m.nesting >= 2);
    }

    #[test]
    fn extracts_extends_and_implements() {
        let facts = extract(
            r#"
            class Base {}
            interface IFace {}
            class Child extends Base implements IFace {}
            "#,
        );
        assert!(facts
            .refs
            .iter()
            .any(|r| r.kind == RefKind::Extends && r.name == "Base"));
        assert!(facts
            .refs
            .iter()
            .any(|r| r.kind == RefKind::Implements && r.name == "IFace"));
    }

    #[test]
    fn never_panics_on_garbage() {
        let _ = extract("function ( { [ unbalanced");
        let _ = extract("\u{0}\u{1}\u{2}");
        let _ = extract("");
    }
}
