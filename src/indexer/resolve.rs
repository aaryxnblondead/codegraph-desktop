//! Resolution of raw facts into a `CodeGraph`.
//!
//! Tier 1 resolves file imports/re-exports to project files (confidence 1.0).
//! Tier 2 resolves symbol references: local scope, then import bindings, then
//! a globally unique name. Ambiguous references stay unresolved.

use std::collections::HashMap;
use std::path::Path;

use crate::config::ProjectConfig;
use crate::ids::symbol_id;
use crate::model::{
    FileInfo, FileStatus, Relationship, RelationshipKind, Symbol, SymbolId, SymbolKind,
};

use super::facts::{FileFacts, RefKind};

pub struct Resolved {
    pub symbols: Vec<Symbol>,
    pub relationships: Vec<Relationship>,
    pub files: HashMap<String, FileInfo>,
    pub refs_total: usize,
    pub refs_resolved: usize,
}

const PROBE_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];

pub fn resolve(
    root: &Path,
    facts: &HashMap<String, FileFacts>,
    config: &ProjectConfig,
) -> Resolved {
    let _ = root;
    let mut symbols = Vec::new();
    let mut relationships = Vec::new();
    let mut files = HashMap::new();
    let mut next_rel_id = 1u64;

    // File nodes.
    let mut file_ids: HashMap<String, SymbolId> = HashMap::new();
    for (rel, file_facts) in facts {
        let id = symbol_id(rel, "file", rel, 0);
        let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
        let mut sym = Symbol::new(id.clone(), name, SymbolKind::Module, rel.clone());
        sym.qualified_name = rel.clone();
        sym.lang = lang_of(rel);
        symbols.push(sym);
        file_ids.insert(rel.clone(), id);
        files.insert(
            rel.clone(),
            FileInfo {
                path: rel.clone(),
                lang: lang_of(rel),
                loc: file_facts.metrics.loc,
                symbol_count: file_facts.symbols.len(),
                status: FileStatus::Indexed,
                ignores: file_facts.ignores.clone(),
                complexity_sum: file_facts
                    .metrics
                    .functions
                    .iter()
                    .map(|f| f.cyclomatic)
                    .sum(),
                complexity_max: file_facts
                    .metrics
                    .functions
                    .iter()
                    .map(|f| f.cyclomatic)
                    .max()
                    .unwrap_or(0),
                function_count: file_facts.metrics.functions.len(),
            },
        );
    }

    // Symbol nodes + contains edges.
    let mut local: HashMap<(String, String), SymbolId> = HashMap::new();
    let mut exports: HashMap<(String, String), SymbolId> = HashMap::new();
    let mut by_name: HashMap<String, Vec<SymbolId>> = HashMap::new();
    let methods_by_name: HashMap<String, Vec<SymbolId>> = HashMap::new();
    let mut symbol_ids: HashMap<(String, usize), SymbolId> = HashMap::new();

    for (rel, file_facts) in facts {
        let file_id = file_ids[rel].clone();
        let mut ordinal_seen: HashMap<(String, String), usize> = HashMap::new();
        for (idx, raw) in file_facts.symbols.iter().enumerate() {
            let key = (raw.kind.clone(), raw.qualified_name.clone());
            let ordinal = *ordinal_seen.get(&key).unwrap_or(&0);
            ordinal_seen.insert(key.clone(), ordinal + 1);
            let id = symbol_id(rel, &raw.kind, &raw.qualified_name, ordinal);
            let kind = SymbolKind::parse(&raw.kind);
            let mut sym = Symbol::new(id.clone(), raw.name.clone(), kind, rel.clone());
            sym.qualified_name = raw.qualified_name.clone();
            sym.line = Some(raw.start_line);
            sym.end_line = Some(raw.end_line);
            sym.exported = raw.exported;
            sym.lang = lang_of(rel);
            // Attach per-function metrics to the matching symbol so findings can
            // point at `file::function:line–end`, not just the file.
            if let Some(metric) = file_facts
                .metrics
                .functions
                .iter()
                .find(|f| f.name == raw.name && f.line == raw.start_line)
            {
                sym.complexity = Some(metric.cyclomatic);
                sym.is_render = metric.render;
            }
            symbols.push(sym);
            symbol_ids.insert((rel.clone(), idx), id.clone());
            local
                .entry((rel.clone(), raw.name.clone()))
                .or_insert_with(|| id.clone());
            if raw.exported {
                exports
                    .entry((rel.clone(), raw.name.clone()))
                    .or_insert_with(|| id.clone());
            }
            by_name
                .entry(raw.name.clone())
                .or_default()
                .push(id.clone());

            relationships.push(Relationship::indexed(
                next_rel_id,
                file_id.clone(),
                id,
                RelationshipKind::Contains,
                Some(raw.start_line),
                1.0,
                "contains",
            ));
            next_rel_id += 1;
        }
    }

    // Tier 1: imports between files.
    let keys: Vec<String> = facts.keys().cloned().collect();
    let rust_modules = rust_module_map(&keys);
    let crate_alias = package_name(root);
    let mut import_targets: HashMap<(String, String), String> = HashMap::new();
    for (rel, file_facts) in facts {
        let file_id = &file_ids[rel];
        for imp in &file_facts.imports {
            let mut targets: Vec<String> = Vec::new();
            let dots_only = !imp.specifier.is_empty() && imp.specifier.chars().all(|c| c == '.');
            if rel.ends_with(".py") && dots_only {
                // `from . import x` imports the submodule `x`, not the package.
                for name in &imp.names {
                    let spec = format!("{}{}", imp.specifier, name);
                    if let Some(t) = resolve_specifier(
                        rel,
                        &spec,
                        facts,
                        &keys,
                        config,
                        &rust_modules,
                        crate_alias.as_deref(),
                    ) {
                        targets.push(t);
                    }
                }
            } else if let Some(target) = resolve_specifier(
                rel,
                &imp.specifier,
                facts,
                &keys,
                config,
                &rust_modules,
                crate_alias.as_deref(),
            ) {
                targets.push(target);
            }

            for target in targets {
                import_targets
                    .entry((rel.clone(), imp.specifier.clone()))
                    .or_insert_with(|| target.clone());
                if let Some(target_id) = file_ids.get(&target) {
                    relationships.push(Relationship::indexed(
                        next_rel_id,
                        file_id.clone(),
                        target_id.clone(),
                        RelationshipKind::Imports,
                        Some(imp.line),
                        1.0,
                        "import",
                    ));
                    next_rel_id += 1;
                }
            }
        }
    }

    // Tier 2: symbol references.
    let mut refs_total = 0usize;
    let mut refs_resolved = 0usize;
    for (rel, file_facts) in facts {
        let file_id = &file_ids[rel];
        for raw_ref in &file_facts.refs {
            refs_total += 1;
            let first = raw_ref
                .name
                .split(['.', ':'])
                .find(|s| !s.is_empty())
                .unwrap_or(&raw_ref.name)
                .to_string();

            let source_id = raw_ref
                .enclosing
                .and_then(|i| symbol_ids.get(&(rel.clone(), i)).cloned())
                .unwrap_or_else(|| file_id.clone());

            let resolved = resolve_ref(
                rel,
                &first,
                &raw_ref.name,
                file_facts,
                &import_targets,
                &local,
                &exports,
                &by_name,
                &methods_by_name,
            )
            .or_else(|| {
                // Qualified Rust paths (`crate::`, `self::`, `super::`) point at
                // a file; pick the named item in it when we can.
                if !rel.ends_with(".rs") {
                    return None;
                }
                let file = resolve_rust(rel, &raw_ref.name, &rust_modules, crate_alias.as_deref())?;
                let item = raw_ref
                    .name
                    .rsplit("::")
                    .next()
                    .unwrap_or(&raw_ref.name)
                    .to_string();
                if let Some(id) = exports.get(&(file.clone(), item.clone())) {
                    return Some((id.clone(), 0.95, "path"));
                }
                if let Some(id) = local.get(&(file.clone(), item)) {
                    return Some((id.clone(), 0.9, "path"));
                }
                file_ids
                    .get(&file)
                    .map(|id| (id.clone(), 0.85, "path-file"))
            });

            if let Some((target, confidence, provenance)) = resolved {
                relationships.push(Relationship::indexed(
                    next_rel_id,
                    source_id,
                    target,
                    edge_kind(raw_ref.kind),
                    Some(raw_ref.line),
                    confidence,
                    provenance,
                ));
                next_rel_id += 1;
                refs_resolved += 1;
            }
        }
    }

    Resolved {
        symbols,
        relationships,
        files,
        refs_total,
        refs_resolved,
    }
}

fn edge_kind(kind: RefKind) -> RelationshipKind {
    match kind {
        RefKind::Call => RelationshipKind::Calls,
        RefKind::Type => RelationshipKind::TypeRef,
        RefKind::Extends => RelationshipKind::Extends,
        RefKind::Implements => RelationshipKind::Implements,
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_ref(
    rel: &str,
    first: &str,
    full: &str,
    file_facts: &FileFacts,
    import_targets: &HashMap<(String, String), String>,
    local: &HashMap<(String, String), SymbolId>,
    exports: &HashMap<(String, String), SymbolId>,
    by_name: &HashMap<String, Vec<SymbolId>>,
    methods_by_name: &HashMap<String, Vec<SymbolId>>,
) -> Option<(SymbolId, f32, &'static str)> {
    // 1. Local scope in the same file.
    if let Some(id) = local.get(&(rel.to_string(), first.to_string())) {
        return Some((id.clone(), 1.0, "local"));
    }

    // 2. Import bindings.
    for imp in &file_facts.imports {
        let Some(target_file) = import_targets.get(&(rel.to_string(), imp.specifier.clone()))
        else {
            continue;
        };
        // Namespace import: `ns.foo`.
        if imp.namespace.as_deref() == Some(first) {
            let member = full.split(['.', ':']).nth(1).unwrap_or(full).to_string();
            if let Some(id) = exports
                .get(&(target_file.clone(), member.clone()))
                .or_else(|| local.get(&(target_file.clone(), member)))
            {
                return Some((id.clone(), 0.9, "namespace"));
            }
        }
        for (i, local_name) in imp.local_names.iter().enumerate() {
            if local_name == first {
                let imported = imp.names.get(i).unwrap_or(local_name);
                if let Some(id) = exports
                    .get(&(target_file.clone(), imported.clone()))
                    .or_else(|| local.get(&(target_file.clone(), imported.clone())))
                {
                    return Some((id.clone(), 0.95, "import-binding"));
                }
            }
        }
    }

    // 3. Globally unique name.
    if let Some(ids) = by_name.get(first) {
        if ids.len() == 1 {
            return Some((ids[0].clone(), 0.4, "unique-name"));
        }
    }

    // 4. Method call on an unknown receiver: only when the method name is
    //    globally unique. Deliberately low confidence.
    if let Some(ids) = methods_by_name.get(first) {
        if ids.len() == 1 {
            return Some((ids[0].clone(), 0.3, "method-name"));
        }
    }
    None
}

/// Resolve an import specifier to a project-relative file, if it is internal.
fn resolve_specifier(
    importer: &str,
    spec: &str,
    facts: &HashMap<String, FileFacts>,
    keys: &[String],
    config: &ProjectConfig,
    rust_modules: &HashMap<String, String>,
    crate_alias: Option<&str>,
) -> Option<String> {
    if importer.ends_with(".py") {
        return resolve_python(importer, spec, facts, keys);
    }
    if importer.ends_with(".rs") {
        return resolve_rust(importer, spec, rust_modules, crate_alias);
    }
    if spec.starts_with('.') {
        let dir = importer.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let joined = normalize_join(dir, spec);
        return probe(&joined, facts, keys);
    }

    // Alias/absolute: try the specifier and a few common roots.
    let mut candidates = vec![spec.to_string()];
    if let Some(rest) = spec.strip_prefix("@/") {
        candidates.push(rest.to_string());
        candidates.push(format!("src/{rest}"));
    }
    candidates.push(format!("src/{spec}"));
    if let Some(base) = &config.edge_weights.get("__base_url") {
        let _ = base;
    }
    for cand in candidates {
        let cand = cand.trim_start_matches('/').to_string();
        if let Some(hit) = probe(&cand, facts, keys) {
            return Some(hit);
        }
    }
    None
}

fn probe(base: &str, facts: &HashMap<String, FileFacts>, keys: &[String]) -> Option<String> {
    let mut cands = Vec::new();
    cands.push(base.to_string());
    for ext in PROBE_EXTS {
        cands.push(format!("{base}.{ext}"));
    }
    for ext in PROBE_EXTS {
        cands.push(format!("{base}/index.{ext}"));
    }
    cands
        .into_iter()
        .find(|c| facts.contains_key(c.as_str()) && keys.iter().any(|k| k == c))
}

/// Resolve a Python import specifier: `a.b`, `.mod`, `..other`.
///
/// Leading dots are relative to the importing package; a bare module is tried
/// from the project root and from `src/` (src-layout projects).
fn resolve_python(
    importer: &str,
    spec: &str,
    facts: &HashMap<String, FileFacts>,
    keys: &[String],
) -> Option<String> {
    let level = spec.chars().take_while(|c| *c == '.').count();
    let module = spec[level..].replace('.', "/");
    // Absolute imports resolve from the project root; relative ones from the
    // importing package's directory.
    let mut base = if level == 0 {
        String::new()
    } else {
        importer
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default()
    };
    for _ in 1..level {
        base = base
            .rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();
    }
    let candidate = match (base.is_empty(), module.is_empty()) {
        (true, true) => return None,
        (true, false) => module.clone(),
        (false, true) => base.clone(),
        (false, false) => format!("{base}/{module}"),
    };
    let mut cands = vec![
        format!("{candidate}.py"),
        format!("{candidate}/__init__.py"),
    ];
    if level == 0 {
        cands.push(format!("src/{candidate}.py"));
        cands.push(format!("src/{candidate}/__init__.py"));
    }
    cands
        .into_iter()
        .find(|c| facts.contains_key(c.as_str()) && keys.iter().any(|k| k == c))
}

/// Map `crate::a::b` module paths to relative files (first wins, so `lib.rs`
/// beats `main.rs` for the crate root).
fn rust_module_map(keys: &[String]) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for key in keys {
        if !key.ends_with(".rs") {
            continue;
        }
        let parts: Vec<&str> = key.split('/').collect();
        let Some(src_idx) = parts.iter().position(|p| *p == "src") else {
            continue;
        };
        let tail = &parts[src_idx + 1..];
        let Some(file) = tail.last() else { continue };
        let Some(stem) = file.strip_suffix(".rs") else {
            continue;
        };
        let mut segs: Vec<String> = vec!["crate".to_string()];
        for seg in &tail[..tail.len() - 1] {
            segs.push((*seg).to_string());
        }
        if !matches!(stem, "lib" | "main" | "mod") {
            segs.push(stem.to_string());
        }
        map.entry(segs.join("::")).or_insert_with(|| key.clone());
    }
    map
}

/// Module path of the file that contains `importer`.
fn rust_file_module(importer: &str) -> Option<Vec<String>> {
    let parts: Vec<&str> = importer.split('/').collect();
    let src_idx = parts.iter().position(|p| *p == "src")?;
    let tail = &parts[src_idx + 1..];
    let file = tail.last()?;
    let stem = file.strip_suffix(".rs")?;
    let mut segs = vec!["crate".to_string()];
    for seg in &tail[..tail.len() - 1] {
        segs.push((*seg).to_string());
    }
    if !matches!(stem, "lib" | "main" | "mod") {
        segs.push(stem.to_string());
    }
    Some(segs)
}

/// Resolve `use crate::…` / `self::…` / `super::…` to a project file.
///
/// The longest path prefix that names a module wins; items after it are
/// symbols inside that file. Paths starting with anything else are external
/// crates and stay unresolved.
fn resolve_rust(
    importer: &str,
    spec: &str,
    modules: &HashMap<String, String>,
    crate_alias: Option<&str>,
) -> Option<String> {
    let mut segs: Vec<String> = spec
        .split("::")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if segs.is_empty() || modules.is_empty() {
        return None;
    }
    let mut prefix: Vec<String> = Vec::new();
    match segs.first().map(String::as_str) {
        Some("crate") => {
            prefix.push("crate".to_string());
            segs.remove(0);
        }
        Some("self") => {
            prefix = rust_file_module(importer)?;
            segs.remove(0);
        }
        Some("super") => {
            prefix = rust_file_module(importer)?;
            while segs.first().map(String::as_str) == Some("super") {
                // Leave at least `crate` in the prefix.
                if prefix.len() > 1 {
                    prefix.pop();
                }
                segs.remove(0);
            }
        }
        other => {
            // A binary in the same package refers to its library by package
            // name (`codegraph_desktop::…`), which is the crate root.
            match crate_alias {
                Some(alias) if other == Some(alias) => {
                    prefix.push("crate".to_string());
                    segs.remove(0);
                }
                _ => return None,
            }
        }
    }

    for take in (0..=segs.len()).rev() {
        let mut cand = prefix.clone();
        cand.extend(segs[..take].iter().cloned());
        if let Some(file) = modules.get(&cand.join("::")) {
            return Some(file.clone());
        }
    }
    None
}

/// Join a base directory and a relative specifier, resolving `.`/`..`.
fn normalize_join(base_dir: &str, spec: &str) -> String {
    let mut parts: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn lang_of(rel: &str) -> Option<String> {
    let ext = rel.rsplit('.').next()?.to_lowercase();
    let lang = match ext.as_str() {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => "typescript",
        "py" => "python",
        "rs" => "rust",
        _ => return None,
    };
    Some(lang.to_string())
}

/// Package name from the project root's `Cargo.toml`, if any.
fn package_name(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = line.strip_prefix("name") {
                if let Some(value) = rest.trim_start().strip_prefix('=') {
                    let name = value.trim().trim_matches(|c| c == '"' || c == '\'').trim();
                    if !name.is_empty() {
                        // Cargo allows hyphens; Rust paths use underscores.
                        return Some(name.replace('-', "_"));
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::facts::{FileFacts, RawImport, RawRef, RawSymbol};

    fn facts_with(
        symbols: Vec<RawSymbol>,
        imports: Vec<RawImport>,
        refs: Vec<RawRef>,
    ) -> FileFacts {
        FileFacts {
            symbols,
            imports,
            refs,
            ..Default::default()
        }
    }

    fn sym(name: &str, exported: bool) -> RawSymbol {
        RawSymbol {
            kind: "function".into(),
            name: name.into(),
            qualified_name: name.into(),
            start_line: 1,
            end_line: 1,
            exported,
            parent: None,
        }
    }

    #[test]
    fn resolves_relative_import_and_symbol_binding() {
        let mut facts = HashMap::new();
        facts.insert(
            "src/a.ts".to_string(),
            facts_with(vec![sym("foo", true)], vec![], vec![]),
        );
        facts.insert(
            "src/b.ts".to_string(),
            facts_with(
                vec![sym("bar", false)],
                vec![RawImport {
                    specifier: "./a".into(),
                    names: vec!["foo".into()],
                    local_names: vec!["foo".into()],
                    namespace: None,
                    default: None,
                    is_reexport: false,
                    line: 1,
                }],
                vec![RawRef {
                    kind: RefKind::Call,
                    name: "foo".into(),
                    enclosing: Some(0),
                    line: 2,
                }],
            ),
        );
        let resolved = resolve(Path::new("."), &facts, &ProjectConfig::default());
        assert_eq!(resolved.refs_total, 1);
        assert_eq!(resolved.refs_resolved, 1);
        assert!(resolved
            .relationships
            .iter()
            .any(|r| r.kind == RelationshipKind::Imports));
        assert!(resolved
            .relationships
            .iter()
            .any(|r| r.kind == RelationshipKind::Calls && r.provenance == "import-binding"));
    }

    #[test]
    fn ambiguous_names_stay_unresolved() {
        let mut facts = HashMap::new();
        facts.insert(
            "a.ts".into(),
            facts_with(vec![sym("dup", true)], vec![], vec![]),
        );
        facts.insert(
            "b.ts".into(),
            facts_with(vec![sym("dup", true)], vec![], vec![]),
        );
        facts.insert(
            "c.ts".into(),
            facts_with(
                vec![sym("user", false)],
                vec![],
                vec![RawRef {
                    kind: RefKind::Call,
                    name: "dup".into(),
                    enclosing: Some(0),
                    line: 1,
                }],
            ),
        );
        let resolved = resolve(Path::new("."), &facts, &ProjectConfig::default());
        assert_eq!(resolved.refs_resolved, 0);
    }

    #[test]
    fn index_probe_finds_files() {
        let mut facts = HashMap::new();
        facts.insert("src/a/index.ts".into(), FileFacts::default());
        let keys: Vec<String> = facts.keys().cloned().collect();
        assert_eq!(
            probe("src/a", &facts, &keys),
            Some("src/a/index.ts".to_string())
        );
    }

    #[test]
    fn python_relative_and_absolute_imports_resolve() {
        let mut facts = HashMap::new();
        for f in [
            "pkg/__init__.py",
            "pkg/mod.py",
            "pkg/sub/__init__.py",
            "pkg/sub/deep.py",
            "src/srclayout.py",
        ] {
            facts.insert(f.to_string(), FileFacts::default());
        }
        let keys: Vec<String> = facts.keys().cloned().collect();

        // `from .mod import x` inside pkg/deep? -> same package module.
        let got = resolve_specifier(
            "pkg/sub/deep.py",
            ".mod",
            &facts,
            &keys,
            &ProjectConfig::default(),
            &HashMap::new(),
            None,
        );
        assert_eq!(got, None, "no pkg/sub/mod.py");

        // `from .. import sibling` style: one level up.
        let got = resolve_specifier(
            "pkg/sub/deep.py",
            "..mod",
            &facts,
            &keys,
            &ProjectConfig::default(),
            &HashMap::new(),
            None,
        );
        assert_eq!(got, Some("pkg/mod.py".to_string()));

        // Package import resolves to its __init__.py.
        let got = resolve_specifier(
            "pkg/mod.py",
            ".sub",
            &facts,
            &keys,
            &ProjectConfig::default(),
            &HashMap::new(),
            None,
        );
        assert_eq!(got, Some("pkg/sub/__init__.py".to_string()));

        // Absolute import from the project root and from src/.
        let got = resolve_specifier(
            "pkg/mod.py",
            "pkg.sub",
            &facts,
            &keys,
            &ProjectConfig::default(),
            &HashMap::new(),
            None,
        );
        assert_eq!(got, Some("pkg/sub/__init__.py".to_string()));
        let got = resolve_specifier(
            "pkg/mod.py",
            "srclayout",
            &facts,
            &keys,
            &ProjectConfig::default(),
            &HashMap::new(),
            None,
        );
        assert_eq!(got, Some("src/srclayout.py".to_string()));
    }

    #[test]
    fn rust_use_paths_resolve_to_module_files() {
        let mut facts = HashMap::new();
        for f in [
            "src/lib.rs",
            "src/metrics.rs",
            "src/graph.rs",
            "src/indexer/mod.rs",
            "src/indexer/resolve.rs",
            "src/indexer/lang/mod.rs",
            "src/indexer/lang/rust_lang.rs",
        ] {
            facts.insert(f.to_string(), FileFacts::default());
        }
        let keys: Vec<String> = facts.keys().cloned().collect();
        let modules = rust_module_map(&keys);
        let cfg = ProjectConfig::default();
        let resolve = |importer: &str, spec: &str| {
            resolve_specifier(
                importer,
                spec,
                &facts,
                &keys,
                &cfg,
                &modules,
                Some("my_crate"),
            )
        };

        assert_eq!(
            resolve("src/main.rs", "crate::metrics::compute"),
            Some("src/metrics.rs".to_string())
        );
        assert_eq!(
            resolve("src/indexer/resolve.rs", "crate::graph::CodeGraph"),
            Some("src/graph.rs".to_string())
        );
        // `self::` stays in the same file.
        assert_eq!(
            resolve("src/indexer/resolve.rs", "self::probe"),
            Some("src/indexer/resolve.rs".to_string())
        );
        // `super::` climbs one module.
        assert_eq!(
            resolve("src/indexer/resolve.rs", "super::super::graph::CodeGraph"),
            Some("src/graph.rs".to_string())
        );
        // Nested module path.
        assert_eq!(
            resolve(
                "src/indexer/mod.rs",
                "crate::indexer::lang::rust_lang::RustExtractor"
            ),
            Some("src/indexer/lang/rust_lang.rs".to_string())
        );
        // External crates stay unresolved.
        assert_eq!(resolve("src/main.rs", "std::collections::HashMap"), None);
        assert_eq!(resolve("src/main.rs", "serde::Deserialize"), None);
        // A binary refers to its own library by package name.
        assert_eq!(
            resolve("src/main.rs", "my_crate::graph::CodeGraph"),
            Some("src/graph.rs".to_string())
        );
    }

    #[test]
    fn python_from_dot_import_creates_edges_to_submodule() {
        let mut facts = HashMap::new();
        facts.insert("pkg/__init__.py".into(), FileFacts::default());
        facts.insert(
            "pkg/mod.py".into(),
            FileFacts {
                symbols: vec![sym("helper", true)],
                ..Default::default()
            },
        );
        facts.insert(
            "pkg/app.py".into(),
            FileFacts {
                imports: vec![RawImport {
                    specifier: ".".into(),
                    names: vec!["mod".into()],
                    local_names: vec!["mod".into()],
                    namespace: None,
                    default: None,
                    is_reexport: false,
                    line: 1,
                }],
                ..Default::default()
            },
        );
        let resolved = resolve(Path::new("."), &facts, &ProjectConfig::default());
        let edges: Vec<&Relationship> = resolved
            .relationships
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .collect();
        assert!(
            !edges.is_empty(),
            "`from . import mod` should produce an import edge"
        );
    }
}
