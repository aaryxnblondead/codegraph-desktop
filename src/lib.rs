//! Core domain model, error handling, storage abstraction, in-memory graph,
//! configuration, and (later) the self-indexer for the CodeGraph desktop
//! explorer.
//!
//! Design notes (rust-engineer):
//! - Owned data flows through the API; no borrows from a storage connection.
//! - `Storage` is a trait with an associated `Error`.
//! - `SymbolId` is a branded newtype.
//! - Errors are explicit via `thiserror`; no `unwrap()` in production code.

pub mod analysis;
pub mod baseline;
pub mod config;
pub mod dsm;
pub mod error;
pub mod git;
pub mod graph;
pub mod graph_view;
pub mod ids;
pub mod impact;
pub mod indexer;
pub mod metrics;
pub mod model;
pub mod modules;
pub mod paths;
pub mod report;
pub mod settings;
pub mod storage;
pub mod structure;
pub mod ui;
pub mod warnings;
pub mod watch;

pub use analysis::{coupling_report, find_cycles, Cycle, FileMetrics};
pub use config::{ModuleDefConfig, ProjectConfig, ProjectKind};
pub use error::CodegraphError;
pub use graph::CodeGraph;
pub use graph_view::GraphView;
pub use model::{
    EdgeDirection, FileInfo, FileStatus, Relationship, RelationshipKind, Symbol, SymbolId,
    SymbolKind,
};
pub use settings::Settings;
pub use storage::{IndexStorage, MemoryStorage, Storage};

pub use storage::SqliteStorage;
