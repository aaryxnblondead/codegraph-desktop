//! Module assignment from user-defined globs, with a derived fallback.

use std::collections::HashMap;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::config::ModuleDefConfig;

/// One configured module.
pub struct ModuleRule {
    pub name: String,
    pub matcher: GlobSet,
    pub allowed: Option<Vec<String>>,
}

/// Assigns files to modules by first matching glob.
#[derive(Default)]
pub struct ModuleAssigner {
    rules: Vec<ModuleRule>,
    configured: bool,
}

impl ModuleAssigner {
    pub fn new(defs: &[ModuleDefConfig]) -> Self {
        let mut rules = Vec::new();
        for def in defs {
            let mut builder = GlobSetBuilder::new();
            let mut ok = false;
            for p in &def.paths {
                // `literal_separator` so `*` stays inside one path segment and
                // `**` is the only thing that crosses directories.
                if let Ok(glob) = GlobBuilder::new(p).literal_separator(true).build() {
                    builder.add(glob);
                    ok = true;
                }
            }
            if !ok {
                continue;
            }
            if let Ok(matcher) = builder.build() {
                rules.push(ModuleRule {
                    name: def.name.clone(),
                    matcher,
                    allowed: def.allowed_deps.clone(),
                });
            }
        }
        let configured = !rules.is_empty();
        Self { rules, configured }
    }

    pub fn is_configured(&self) -> bool {
        self.configured
    }

    /// Module name for a normalized relative path.
    pub fn module_of(&self, path: &str) -> String {
        if !self.configured {
            return crate::graph_view::module_key(path);
        }
        self.matching(path)
            .into_iter()
            .next()
            .unwrap_or_else(|| "Unassigned".to_string())
    }

    /// All modules whose globs match `path` (to detect overlaps).
    pub fn matching(&self, path: &str) -> Vec<String> {
        self.rules
            .iter()
            .filter(|r| r.matcher.is_match(path))
            .map(|r| r.name.clone())
            .collect()
    }

    /// Allowed dependency targets for a module, if restricted.
    pub fn allowed(&self, module: &str) -> Option<&[String]> {
        self.rules
            .iter()
            .find(|r| r.name == module)
            .and_then(|r| r.allowed.as_deref())
    }

    /// Files matched by more than one module (config warnings).
    pub fn overlaps(&self, paths: &[String]) -> Vec<(String, Vec<String>)> {
        paths
            .iter()
            .filter_map(|p| {
                let m = self.matching(p);
                if m.len() > 1 {
                    Some((p.clone(), m))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Module names for every file path.
    pub fn assign_all(&self, paths: &[String]) -> HashMap<String, String> {
        paths
            .iter()
            .map(|p| (p.clone(), self.module_of(p)))
            .collect()
    }
}

/// Starter config: one module per top-level folder, plus one for root files.
pub fn starter_modules(folders: &[String]) -> Vec<ModuleDefConfig> {
    let mut out: Vec<ModuleDefConfig> = folders
        .iter()
        .map(|folder| ModuleDefConfig {
            name: folder.clone(),
            paths: vec![format!("{folder}/**")],
            allowed_deps: None,
        })
        .collect();
    out.push(ModuleDefConfig {
        name: "Root files".to_string(),
        paths: vec!["*".to_string()],
        allowed_deps: None,
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, paths: &[&str], allowed: Option<&[&str]>) -> ModuleDefConfig {
        ModuleDefConfig {
            name: name.into(),
            paths: paths.iter().map(|p| p.to_string()).collect(),
            allowed_deps: allowed.map(|a| a.iter().map(|s| s.to_string()).collect()),
        }
    }

    #[test]
    fn first_match_wins_and_parentheses_are_literal() {
        let a = def("Admin", &["app/(admin)/**"], Some(&["Shared"]));
        let b = def("UI", &["app/**"], None);
        let assigner = ModuleAssigner::new(&[a, b]);
        assert_eq!(assigner.module_of("app/(admin)/page.tsx"), "Admin");
        assert_eq!(assigner.module_of("app/other/page.tsx"), "UI");
        assert_eq!(assigner.module_of("lib/x.ts"), "Unassigned");
    }

    #[test]
    fn derived_fallback_when_unconfigured() {
        let assigner = ModuleAssigner::new(&[]);
        assert!(!assigner.is_configured());
        assert_eq!(
            assigner.module_of("src/components/ui/button.tsx"),
            "components/ui"
        );
    }

    #[test]
    fn overlap_detection() {
        let a = def("A", &["app/**"], None);
        let b = def("B", &["app/**"], None);
        let assigner = ModuleAssigner::new(&[a, b]);
        let overlaps = assigner.overlaps(&["app/x.ts".to_string()]);
        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0].1.len(), 2);
    }

    #[test]
    fn starter_modules_cover_folders_and_root_files() {
        let defs = starter_modules(&["src".to_string(), "tests".to_string()]);
        let assigner = ModuleAssigner::new(&defs);
        assert_eq!(assigner.module_of("src/main.rs"), "src");
        assert_eq!(assigner.module_of("tests/cli.rs"), "tests");
        assert_eq!(assigner.module_of("Cargo.toml"), "Root files");
        assert_eq!(assigner.module_of("docs/METRICS.md"), "Unassigned");
    }

    #[test]
    fn allowed_deps() {
        let a = def("UI", &["app/**"], Some(&["Domain"]));
        let assigner = ModuleAssigner::new(&[a]);
        assert_eq!(
            assigner.allowed("UI").map(|s| s.to_vec()),
            Some(vec!["Domain".to_string()])
        );
        assert!(assigner.allowed("Nope").is_none());
    }
}
