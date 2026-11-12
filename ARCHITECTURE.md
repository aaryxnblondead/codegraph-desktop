# Architecture (baseline)

`codegraph_desktop` is a Rust workspace crate: a GUI-free library plus an egui/eframe binary.

## Library (`src/lib.rs`)

- `model` — `Symbol`, `SymbolId` (String newtype), `SymbolKind`, `Relationship`, `RelationshipKind`, `EdgeDirection`.
- `error` — `CodegraphError` (thiserror): Database/SymbolNotFound/DuplicateSymbolId/Schema/Io.
- `graph` — `CodeGraph`: owned `Vec<Symbol>` + `Vec<Relationship>` plus `HashMap` indexes (`by_id`, `outgoing`, `incoming`); `find`, `symbol`, `relationships_for`, `neighbors_in/out`.
- `storage` — `Storage` trait (associated `Error`), `MemoryStorage`, `SqliteStorage` (legacy `codegraph.db`: tables `nodes`/`edges`).
- `analysis` — `find_cycles` (Tarjan SCC), `coupling_report` (Ca/Ce/instability/cohesion), `summarize`.
- `graph_view` — `GraphView` (immutable: nodes, resolved edge index pairs, precomputed positions, sizes), `layered_layout` (SCC condensation + longest path + barycenter), `compute_pagerank`, `build_module_overview`, `build_file_overview`, `build_ego_view`, `module_key`, `top_edges`. No GUI types.
- `main` — egui/eframe app: header, left symbol list, central custom graph painter (`draw_graph`, `draw_arrow`), bottom audit/detail panel. Async DB load via `std::thread` + `mpsc`.

## Invariants

- Lib is GUI-free.
- `GraphView` is immutable; layout precomputed once; positions finite.
- Draw path is pure; bounded aggregates only.
- Heavy work off the UI thread.

## Planned extensions

Self-indexing (walk/parse/resolve/cache), metrics/structure/impact/DSM, git analytics, warnings/baselines, report + CLI. See `docs/superpowers/plans/2026-11-11-codegraph-self-indexing.md`.
