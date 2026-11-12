//! Custom error type for the CodeGraph desktop application.
//!
//! Every error variant carries context (ids, paths, reasons) so callers can
//! render actionable messages instead of generic "something went wrong"
//! strings. Errors propagate with `?` via `#[from]`.

use crate::model::SymbolId;

/// All failures the CodeGraph explorer can produce.
#[derive(Debug, thiserror::Error)]
pub enum CodegraphError {
    /// The SQLite database could not be opened or a query failed.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// A requested symbol does not exist in the graph.
    #[error("symbol not found: {id}")]
    SymbolNotFound { id: SymbolId },

    /// A symbol with this id already exists (used when building indexes).
    #[error("duplicate symbol id: {id}")]
    DuplicateSymbolId { id: SymbolId },

    /// The database schema does not match what this build expects.
    #[error("schema error: {reason}")]
    Schema { reason: String },

    /// A file could not be read (e.g. an exported graph snapshot).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Indexing failed (walk, cache, or pipeline).
    #[error("index error: {0}")]
    Index(String),

    /// Configuration is invalid.
    #[error("config error: {0}")]
    Config(String),

    /// A git command failed or git is unavailable.
    #[error("git error: {0}")]
    Git(String),

    /// A source file could not be parsed.
    #[error("parse error in {path}: {reason}")]
    Parse { path: String, reason: String },
}

impl CodegraphError {
    /// Whether this error means "the requested data simply isn't there" (as
    /// opposed to a real failure such as a corrupt database).
    pub fn is_not_found(&self) -> bool {
        matches!(self, CodegraphError::SymbolNotFound { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_detection() {
        let not_found = CodegraphError::SymbolNotFound {
            id: SymbolId("9".into()),
        };
        assert!(not_found.is_not_found());

        let schema = CodegraphError::Schema {
            reason: "missing symbols table".into(),
        };
        assert!(!schema.is_not_found());
    }
}
