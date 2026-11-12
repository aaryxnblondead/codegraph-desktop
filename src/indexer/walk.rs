//! File discovery for indexing.
//!
//! Respects `.gitignore` and hidden files, applies default ignores
//! (`node_modules`, `target`, …), extra user globs, and a 1 MB size cap.

use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

const MAX_BYTES: u64 = 1_000_000;

const DEFAULT_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    "dist",
    "build",
    ".next",
    "out",
    "venv",
    ".venv",
    "__pycache__",
    "coverage",
];

const DEFAULT_FILE_SUFFIXES: &[&str] = &[".min.js", ".d.ts", ".map"];

#[derive(Debug, Default)]
pub struct Discovered {
    pub files: Vec<PathBuf>,
    pub skipped_large: Vec<PathBuf>,
    pub skipped_ignored: Vec<PathBuf>,
}

/// Walk `root`, returning files to index and those skipped.
pub fn walk_files(root: &Path, extra_ignore: &[String]) -> Discovered {
    let extra = build_globset(extra_ignore);
    let mut out = Discovered::default();

    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .build();

    for entry in walker.flatten() {
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if is_default_ignored(path) {
            out.skipped_ignored.push(path.to_path_buf());
            continue;
        }
        if let Some(set) = &extra {
            let rel = path.strip_prefix(root).unwrap_or(path);
            if set.is_match(rel) {
                out.skipped_ignored.push(path.to_path_buf());
                continue;
            }
        }
        match entry.metadata() {
            Ok(meta) if meta.len() > MAX_BYTES => out.skipped_large.push(path.to_path_buf()),
            Ok(_) => out.files.push(path.to_path_buf()),
            Err(_) => out.skipped_ignored.push(path.to_path_buf()),
        }
    }

    out.files.sort();
    out
}

fn build_globset(patterns: &[String]) -> Option<GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(glob) = Glob::new(p) {
            builder.add(glob);
        }
    }
    builder.build().ok()
}

fn is_default_ignored(path: &Path) -> bool {
    // Any ancestor directory whose name is a default-ignored folder.
    for component in path.components() {
        if let Some(name) = component.as_os_str().to_str() {
            if DEFAULT_DIRS.contains(&name) {
                return true;
            }
        }
    }
    let s = path.to_string_lossy();
    DEFAULT_FILE_SUFFIXES.iter().any(|suf| s.ends_with(suf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, rel: &str, bytes: usize) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn finds_sources_and_skips_defaults() {
        let dir = std::env::temp_dir().join("cg_walk_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        touch(&dir, "src/a.ts", 10);
        touch(&dir, "src/b.py", 10);
        touch(&dir, "node_modules/x/index.js", 10);
        touch(&dir, "target/debug/x", 10);
        touch(&dir, "src/big.ts", 1_500_000);

        let got = walk_files(&dir, &[]);
        let names: Vec<String> = got
            .files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"a.ts".to_string()));
        assert!(names.contains(&"b.py".to_string()));
        assert!(!names.contains(&"index.js".to_string()));
        assert!(!names.contains(&"big.ts".to_string()));
        assert_eq!(got.skipped_large.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extra_ignore_globs() {
        let dir = std::env::temp_dir().join("cg_walk_extra");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        touch(&dir, "src/a.ts", 10);
        touch(&dir, "generated/b.ts", 10);

        let got = walk_files(&dir, &["generated/**".to_string()]);
        let names: Vec<String> = got
            .files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"a.ts".to_string()));
        assert!(!names.contains(&"b.ts".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
