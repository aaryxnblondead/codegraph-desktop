//! Python extractor using tree-sitter.

use std::path::Path;

use tree_sitter::{Node, Parser};

use super::super::facts::{FileFacts, RawImport, RawRef, RawSymbol, RefKind};
use super::LanguageExtractor;

pub struct PythonExtractor;

impl LanguageExtractor for PythonExtractor {
    fn language_id(&self) -> &'static str {
        "python"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["py"]
    }
    fn extract(&self, _path: &Path, src: &str) -> FileFacts {
        let mut facts = FileFacts::default();
        let language = tree_sitter::Language::new(tree_sitter_python::LANGUAGE);
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
            class_stack: Vec::new(),
        };
        ctx.walk(tree.root_node(), "", false);
        facts
    }
}

struct Ctx<'a> {
    src: &'a str,
    facts: &'a mut FileFacts,
    class_stack: Vec<String>,
}

impl Ctx<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        node.utf8_text(self.src.as_bytes()).unwrap_or("")
    }

    fn line(&self, node: Node<'_>) -> u32 {
        node.start_position().row as u32 + 1
    }

    fn add_symbol(&mut self, kind: &str, name: &str, node: Node<'_>, exported: bool) -> usize {
        let qualified = if self.class_stack.is_empty() {
            name.to_string()
        } else {
            format!("{}::{}", self.class_stack.join("::"), name)
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

    fn walk(&mut self, node: Node<'_>, prefix: &str, exported: bool) {
        match node.kind() {
            "function_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    let kind = if self.class_stack.is_empty() {
                        "function"
                    } else {
                        "method"
                    };
                    let idx = self.add_symbol(kind, &name, node, exported);
                    self.record_metric(node, &name);
                    let body = node.child_by_field_name("body");
                    self.walk_body(body, idx, &format!("{prefix}{name}"));
                }
                return;
            }
            "class_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    let idx = self.add_symbol("class", &name, node, exported);
                    self.class_stack.push(name.clone());
                    if let Some(body) = node.child_by_field_name("body") {
                        let mut cursor = body.walk();
                        for child in body.named_children(&mut cursor) {
                            self.walk(child, prefix, false);
                        }
                    }
                    self.class_stack.pop();
                    let _ = idx;
                }
                return;
            }
            "decorated_definition" => {
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if matches!(child.kind(), "function_definition" | "class_definition") {
                        self.walk(child, prefix, exported);
                    }
                }
                return;
            }
            "import_statement" => {
                self.handle_import(node);
                return;
            }
            "import_from_statement" => {
                self.handle_import_from(node);
                return;
            }
            "call" => {
                if let Some(func) = node.child_by_field_name("function") {
                    let name = self.text(func).to_string();
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
            "class_definition_heritage" | "argument_list" => {}
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child, prefix, exported);
        }
    }

    fn walk_body(&mut self, body: Option<Node<'_>>, _idx: usize, prefix: &str) {
        if let Some(body) = body {
            let mut cursor = body.walk();
            for child in body.named_children(&mut cursor) {
                self.walk(child, prefix, false);
            }
        }
    }

    fn handle_import(&mut self, node: Node<'_>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "dotted_name" => {
                    let spec = self.text(child).to_string();
                    let local = spec.rsplit('.').next().unwrap_or(&spec).to_string();
                    self.facts.imports.push(RawImport {
                        specifier: spec.clone(),
                        names: vec![spec.clone()],
                        local_names: vec![local],
                        namespace: None,
                        default: None,
                        is_reexport: false,
                        line: self.line(node),
                    });
                }
                "aliased_import" => {
                    let name = child
                        .child_by_field_name("name")
                        .map(|n| self.text(n).to_string())
                        .unwrap_or_default();
                    let alias = child
                        .child_by_field_name("alias")
                        .map(|n| self.text(n).to_string())
                        .unwrap_or_else(|| name.clone());
                    if !name.is_empty() {
                        self.facts.imports.push(RawImport {
                            specifier: name.clone(),
                            names: vec![name],
                            local_names: vec![alias],
                            namespace: None,
                            default: None,
                            is_reexport: false,
                            line: self.line(node),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn handle_import_from(&mut self, node: Node<'_>) {
        let module = node
            .child_by_field_name("module_name")
            .map(|n| self.text(n).to_string())
            .or_else(|| {
                node.child_by_field_name("relative_import")
                    .map(|n| self.text(n).to_string())
            })
            .unwrap_or_default();
        let mut names = Vec::new();
        let mut locals = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "dotted_name" => {
                    let t = self.text(child).to_string();
                    names.push(t.clone());
                    locals.push(t);
                }
                "aliased_import" => {
                    let n = child
                        .child_by_field_name("name")
                        .map(|x| self.text(x).to_string())
                        .unwrap_or_default();
                    let a = child
                        .child_by_field_name("alias")
                        .map(|x| self.text(x).to_string())
                        .unwrap_or_else(|| n.clone());
                    if !n.is_empty() {
                        names.push(n);
                        locals.push(a);
                    }
                }
                "identifier" => {
                    let t = self.text(child).to_string();
                    names.push(t.clone());
                    locals.push(t);
                }
                _ => {}
            }
        }
        if !module.is_empty() || !names.is_empty() {
            self.facts.imports.push(RawImport {
                specifier: module,
                names,
                local_names: locals,
                namespace: None,
                default: None,
                is_reexport: false,
                line: self.line(node),
            });
        }
    }

    fn record_metric(&mut self, node: Node<'_>, name: &str) {
        let params = node
            .child_by_field_name("parameters")
            .map(|p| p.named_child_count() as u32)
            .unwrap_or(0);
        let body = node.child_by_field_name("body").unwrap_or(node);
        let (cyclomatic, nesting) = scan_complexity(body);
        let loc = node
            .end_position()
            .row
            .saturating_sub(node.start_position().row)
            + 1;
        self.facts
            .metrics
            .functions
            .push(super::super::facts::FunctionMetric {
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

fn scan_complexity(node: Node<'_>) -> (u32, u32) {
    let mut cyclomatic = 1u32;
    let mut max_depth = 0u32;
    let mut stack = vec![(node, 0u32)];
    while let Some((n, depth)) = stack.pop() {
        let is_decision = matches!(
            n.kind(),
            "if_statement"
                | "elif_clause"
                | "for_statement"
                | "while_statement"
                | "except_clause"
                | "conditional_expression"
                | "boolean_operator"
                | "case_clause"
                | "for_in_clause"
                | "if_clause"
        );
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
        PythonExtractor.extract(&PathBuf::from("pkg/a.py"), src)
    }

    #[test]
    fn extracts_functions_and_classes() {
        let facts = extract(
            "def top(x):\n    return x\n\nclass C:\n    def method(self):\n        return 1\n",
        );
        let names: Vec<&str> = facts.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"top"), "{names:?}");
        assert!(names.contains(&"C"), "{names:?}");
        assert!(names.contains(&"method"), "{names:?}");
        assert!(facts
            .symbols
            .iter()
            .any(|s| s.kind == "method" && s.qualified_name == "C::method"));
    }

    #[test]
    fn extracts_imports() {
        let facts =
            extract("import os\nimport a.b as ab\nfrom . import x\nfrom .mod import y as z\n");
        assert!(facts.imports.iter().any(|i| i.specifier == "os"));
        assert!(facts
            .imports
            .iter()
            .any(|i| i.specifier == "a.b" && i.local_names == vec!["ab".to_string()]));
        assert!(facts.imports.iter().any(|i| i.specifier == "."));
        assert!(facts
            .imports
            .iter()
            .any(|i| i.specifier == ".mod" && i.local_names.contains(&"z".to_string())));
    }

    #[test]
    fn records_complexity() {
        let facts = extract(
            "def f(a):\n    if a and 1:\n        for i in a:\n            pass\n    return a\n",
        );
        let m = facts
            .metrics
            .functions
            .iter()
            .find(|m| m.name == "f")
            .unwrap();
        // 1 + if + and + for = 4
        assert_eq!(m.cyclomatic, 4, "got {}", m.cyclomatic);
        assert!(m.nesting >= 2);
    }

    #[test]
    fn never_panics_on_garbage() {
        let _ = extract("def ( { [");
        let _ = extract("\u{0}\u{1}");
        let _ = extract("");
    }
}
