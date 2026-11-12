//! Stable, content-addressed symbol ids for the indexer.
//!
//! Ids survive re-indexing (baselines depend on this): the hash covers the
//! relative path, kind, qualified name, and a per-file ordinal that
//! disambiguates duplicates by source position.

use crate::model::SymbolId;

/// Build a stable id: first 16 hex chars of blake3 over the key fields.
pub fn symbol_id(rel_path: &str, kind: &str, qualified_name: &str, ordinal: usize) -> SymbolId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(rel_path.as_bytes());
    hasher.update(b"\0");
    hasher.update(kind.as_bytes());
    hasher.update(b"\0");
    hasher.update(qualified_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(&(ordinal as u64).to_le_bytes());
    let hex = hasher.finalize().to_hex().to_string();
    SymbolId(hex.chars().take(16).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let a = symbol_id("src/a.ts", "function", "f", 0);
        let b = symbol_id("src/a.ts", "function", "f", 0);
        assert_eq!(a, b);
        assert_eq!(a.0.len(), 16);
    }

    #[test]
    fn ordinal_disambiguates() {
        let a = symbol_id("src/a.ts", "function", "f", 0);
        let b = symbol_id("src/a.ts", "function", "f", 1);
        assert_ne!(a, b);
    }

    #[test]
    fn path_and_kind_participate() {
        assert_ne!(
            symbol_id("src/a.ts", "function", "f", 0),
            symbol_id("src/b.ts", "function", "f", 0)
        );
        assert_ne!(
            symbol_id("src/a.ts", "function", "f", 0),
            symbol_id("src/a.ts", "class", "f", 0)
        );
    }
}
