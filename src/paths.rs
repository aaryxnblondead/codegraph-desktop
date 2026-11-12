//! Path normalization, project identity, and app data directories.
//!
//! Paths are stored **relative to the project root**, with forward slashes and
//! no Windows extended-length (`\\?\`) prefix. Comparison is case-insensitive
//! on Windows; display keeps the original casing.

use std::path::{Path, PathBuf};

/// Normalize `path` relative to `root` into a stable `a/b/c` form.
///
/// Falls back to the absolute normalized path when `path` is not under `root`.
pub fn normalize(root: &Path, path: &Path) -> String {
    let root = canonical(root);
    let path = canonical(path);
    let rel = path.strip_prefix(&root).unwrap_or(&path);
    to_slashes(rel)
}

/// Normalize an already-relative path (backslashes -> slashes, drop `./`).
pub fn normalize_relative(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = p.strip_prefix("./").unwrap_or(&p);
    p.trim_start_matches('/').to_string()
}

/// Canonicalize without the `\\?\` prefix where possible.
pub fn canonical(path: &Path) -> PathBuf {
    let c = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    strip_verbatim(&c)
}

fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

fn to_slashes(path: &Path) -> String {
    path.components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

/// Stable project id: first 16 hex chars of blake3 of the canonical root.
pub fn project_root_id(root: &Path) -> String {
    let canon = canonical(root);
    let hash = blake3::hash(canon.to_string_lossy().as_bytes());
    hash.to_hex().chars().take(16).collect()
}

/// App data directories (`<config>`, `<cache>`).
pub fn app_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("dev", "codegraph", "codegraph_desktop")
}

/// Case-insensitive equality on Windows, exact elsewhere.
pub fn paths_equal(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_backslashes_and_prefix() {
        assert_eq!(normalize_relative(r"a\b\c.rs"), "a/b/c.rs");
        assert_eq!(normalize_relative("./a/b.rs"), "a/b.rs");
        assert_eq!(normalize_relative("/a/b.rs"), "a/b.rs");
    }

    #[test]
    fn normalize_relative_to_root() {
        let root = std::env::temp_dir().join("cg_norm_root");
        let file = root.join("src").join("main.rs");
        let got = normalize(&root, &file);
        assert_eq!(got, "src/main.rs", "got {got}");
    }

    #[test]
    fn project_id_is_stable_and_distinct() {
        let a = std::env::temp_dir().join("cg_proj_a");
        let b = std::env::temp_dir().join("cg_proj_b");
        assert_eq!(project_root_id(&a), project_root_id(&a));
        assert_ne!(project_root_id(&a), project_root_id(&b));
        assert_eq!(project_root_id(&a).len(), 16);
    }

    #[test]
    fn case_sensitivity_rules() {
        if cfg!(windows) {
            assert!(paths_equal("A/b.rs", "a/B.rs"));
        } else {
            assert!(!paths_equal("A/b.rs", "a/B.rs"));
        }
    }
}
