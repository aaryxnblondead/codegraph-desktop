# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Planned

- Dependency Structure Matrix view
- `cg_index --watch` for headless live indexing
- Light theme with verified contrast

## [0.1.0] — 2026-11-12

First public version.

### Added

- **Self-indexing**: parses TypeScript/JavaScript, Python and Rust with
  tree-sitter; resolves imports and symbols in tiers with explicit confidence.
- **Index cache**: per-file facts keyed by blake3 content hash; 867 files in
  ~20 s cold, ~1.2 s warm, ~190 ms to load the saved graph.
- **Live sync**: file watching with a 600 ms debounce, incremental re-index, and
  "touched" highlights on files, modules and graph nodes.
- **Git awareness** (read-only): branch, HEAD, worktrees, branch list; open a
  worktree as the indexed root; re-index when HEAD moves.
- **Analysis**: module coupling/cohesion (Ca, Ce, instability, abstractness,
  distance), file-level cycles with the cheapest edges to cut, impact closures,
  per-function cyclomatic complexity, git churn hotspots, hidden coupling, and
  layer-rule violations.
- **Findings**: symbol-level locations (`file::name:line–end`), calibrated
  thresholds, JSX-aware complexity, committed + inline suppression.
- **Baselines**: `--save-baseline`, new/fixed deltas in the app and the CLI,
  `--fail-on-new` for CI.
- **Outputs**: deterministic JSON and Markdown reports; clipboard text for
  every pane.
- **CLI**: `cg_index` with `--json`, `--markdown`, `--baseline`,
  `--fail-on-new`, `--save-baseline`, `--min-confidence`, `--no-git`.
- **Accessibility**: custom-painted controls publish AccessKit nodes; rows,
  graph nodes and headers are announced with names, roles and state.
- 141 automated tests; `cargo fmt`, `cargo clippy -D warnings` and
  `cargo build --release` clean.
