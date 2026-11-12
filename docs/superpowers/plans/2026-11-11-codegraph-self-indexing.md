# CodeGraph Desktop: Self-Indexing + Engineering Analysis — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `codegraph_desktop` from a `codegraph.db` viewer into a self-indexing Rust project explorer that also performs engineering analysis (coupling/cohesion, cycles, impact, complexity, git hotspots, hidden coupling, layering, baselines).

**Architecture:** Add a GUI-free lib pipeline (`walk → hash → parse/extract → resolve → cache → CodeGraph`) plus analysis modules (`metrics`, `structure`, `impact`, `dsm`, `git`, `warnings`, `baseline`, `report`). The egui app only builds bounded views and renders; all heavy work runs on worker threads and returns through channels. The legacy `codegraph.db` path is preserved via `SqliteStorage`.

**Tech Stack:** Rust 2021; eframe/egui 0.37; rusqlite 0.31 (bundled); petgraph 0.6; rfd 0.14; thiserror 1.0; proptest 1.4 (dev); add `ignore`, `rayon`, `blake3`, `serde`, `serde_json`, `globset`, `directories`, `dunce`, `clap` (CLI), `tree-sitter` + `tree-sitter-typescript`/`-python`/`-rust`.

**Spec:** The roadmap document supplied by the user (sections 1–9, Appendices A–E). If it is not on disk yet, save it verbatim as `docs/specs/codegraph-roadmap.md` before starting; executors must read it.

## Global Constraints

- Validation after **every** phase: `cargo check`; `cargo clippy --all-targets --all-features` (zero warnings); `cargo fmt --check`; `cargo test --all-features`; `cargo build --release`.
- The lib is GUI-free. No egui types in `src/` outside `main.rs`.
- `GraphView` is immutable data (nodes, resolved edge pairs, precomputed positions/sizes). No interior mutability, no simulation in draw, positions always finite.
- Draw functions are pure; per-frame state (highlight set, hover) is passed as arguments.
- The UI renders bounded aggregates only (`MAX_MODULES=120`, `MAX_FILE_NODES=160`, `MAX_VISUALIZED_NODES=80`, `MAX_LIST=500`).
- All indexing/analysis off the UI thread via worker + channel + `ctx.request_repaint()`.
- Never write inside the indexed project (except a user-created `codegraph.config.json`).
- Never panic on bad input; per-file work wrapped in `catch_unwind`; failed files recorded and skipped.
- Do not run `git commit`/`push`/history-rewriting. Read-only git only.
- Preserve public API where practical; if a signature changes, update every caller and test in the same phase.
- Add only the dependencies in the spec unless justified; log reasons in `DECISIONS.md`; pin mutually compatible tree-sitter versions and prove with `cargo check`.
- Plain-English docs/comments/UI text; short sentences; no marketing tone.

## Review Focus

1. **Legacy DB with unknown kinds** — `nodes.kind`/`edges.kind` strings outside the enum must map to `Other(String)`, never panic. Test with synthetic rows.
2. **Config parse failures** — bad JSON, comments, trailing commas, unknown fields: must not crash; show line/column; keep last good config.
3. **Windows path normalization** — `\\?\` prefix, backslashes, case-insensitivity, relative-to-root storage: stable ids and module globs must match.
4. **Hostile/binary/oversized files** — `catch_unwind`, 1 MB cap, skip with status; never abort the index.
5. **Ambiguous resolution** — same name in multiple files must stay `unique-name` low confidence or unresolved; never silently create a wrong edge.
6. **Cancellation mid-index** — old cache and graph must remain usable; no partial swap.
7. **Git absent / not a repo** — features disable quietly with one summary note; no error dialog.
8. **Metric gaming / false precision** — every number carries provenance/confidence; `min_confidence` filters; docs state formulas are heuristics.

## Coverage Matrix (spec → tasks)

| Spec section | Tasks |
|---|---|
| §3 Invariants | enforced by every task; checked in Phase 9 |
| §5.1 lib layout | Tasks 0.1, 1.1–1.6, 2.1–2.7, 3.1–3.5, 4.1–4.3, 5.1–5.5, 6.1–6.5, 7.1–7.4, 8.1–8.6 |
| §5.2 deps/features | Task 1.0 |
| §5.3 model | Task 1.2 |
| §5.4 indexer pipeline | Tasks 2.1–2.7 |
| §5.5 resolution | Tasks 2.6, 3.1–3.3 |
| §5.6 storage locations | Tasks 1.1, 1.4, 2.3 |
| §5.7 config | Tasks 1.3, 5.1 |
| §6 Phase 0–9 | Phases 0–9 below |
| Appendix A metrics | Tasks 4.1–4.2, 7.2–7.3 |
| Appendix B rules | Task 5.2 |
| Appendix C layer example | Task 5.1 |
| Appendix D resolution details | Tasks 2.6, 3.1–3.3 |
| Appendix E fixtures | Tasks 2.6, 3.1–3.3, 3.5 |

## File Structure

```
src/lib.rs            re-exports; feature gates
src/model.rs          Symbol, SymbolId, SymbolKind, Relationship, RelationshipKind, FileInfo
src/error.rs          Index, Config, Git, Parse variants
src/graph.rs          CodeGraph + files map + edge-kind/confidence queries
src/storage.rs        Storage trait; SqliteStorage (legacy); IndexStorage (own cache)
src/paths.rs          normalize, project_id, app dirs
src/config.rs         ProjectConfig, defaults, line/col errors
src/modules.rs        ModuleDef, ModuleAssigner, overview builder
src/indexer/{mod,walk,cache,facts,resolve,lang/{mod,typescript,python,rust_lang}}.rs
src/metrics.rs        coupling/cohesion/abstractness/complexity roll-ups
src/structure.rs      feedback arc set, articulation, betweenness, reachability
src/impact.rs         impact closure + path finder
src/dsm.rs            DSM builder
src/git.rs            churn + co-change via git CLI
src/warnings.rs       Severity, Rule, Warning, run_warnings
src/baseline.rs       snapshots + diff
src/report.rs         JSON + Markdown export
src/settings.rs       app settings persistence (no GUI)
src/analysis.rs       analyze() -> AnalysisBundle (kept, orchestrates)
src/graph_view.rs     extended builders (filters, highlight, module config)
src/bin/cg_index.rs   headless CLI
src/main.rs           egui app
tests/fixtures/{ts_app,py_pkg,rust_ws}/
docs/METRICS.md, ARCHITECTURE.md, PROGRESS.md, DECISIONS.md
```

---

## Phase 0 — Recon & baseline

### Task 0.1: Record baseline

**Files:** Create `PROGRESS.md`, `DECISIONS.md`, `ARCHITECTURE.md`.

**Interfaces:** none.

- [ ] Run `cargo test --all-features`; record count (expect 22) and the five validation results in `PROGRESS.md`.
- [ ] Read `src/` and list current modules/types in `ARCHITECTURE.md` (short).
- [ ] Create `PROGRESS.md` with a checklist of Phases 0–9 (status `todo`) and `DECISIONS.md` with an empty table header.
- [ ] Commit: `docs: record baseline and phase plan`.

**Gate:** baseline recorded; validation green.

---

## Phase 1 — Foundations

### Task 1.0: Dependencies & features

**Files:** Modify `Cargo.toml`.

**Interfaces:** Produces feature names `lang-typescript`, `lang-python`, `lang-rust`.

- [ ] Add `ignore`, `rayon`, `blake3`, `serde`(derive), `serde_json`, `globset`, `directories`, `dunce`, `clap`(derive, optional bin), `tree-sitter`, `tree-sitter-typescript`, `tree-sitter-python`, `tree-sitter-rust`.
- [ ] `[features] default = ["sqlite","lang-typescript","lang-python","lang-rust"]`; each `lang-* = ["dep:tree-sitter-X"]`.
- [ ] Run `cargo check`; pin versions until compatible; log chosen versions in `DECISIONS.md`.
- [ ] Commit: `build: add indexer and analysis dependencies`.

### Task 1.1: `paths.rs`

**Files:** Create `src/paths.rs`; add `pub mod paths;` to `lib.rs`; Test inline module.

**Interfaces:**
- Produces: `pub fn normalize(path: &Path) -> String` (relative, `/`, no `\\?\`); `pub fn project_root_id(root: &Path) -> String` (16 hex of blake3 of canonicalized root); `pub fn app_dirs() -> ProjectDirs`.

- [ ] Write failing tests: `normalize` on `C:\\a\\b\\c.rs` → `a/b/c.rs`; preserves original case; strips `\\?\`; `project_root_id` stable for the same root and differs for another.
- [ ] Run tests → fail.
- [ ] Implement with `dunce::canonicalize` + `blake3`.
- [ ] Run tests → pass.
- [ ] Commit: `feat(paths): normalization, project id, app dirs`.

### Task 1.2: Model extensions

**Files:** Modify `src/model.rs`, `src/graph.rs`, `src/storage.rs`.

**Interfaces:**
- `SymbolKind`/`RelationshipKind` gain `Other(String)`; `From<&str>` mapping (unknown → `Other`).
- `Symbol { end_line, exported, qualified_name, lang }` (defaults for legacy).
- `Relationship { line, confidence: f32, provenance: String }` (legacy: line None, confidence 1.0, provenance "legacy").
- `FileInfo { path, lang, loc, symbol_count, status }`; `CodeGraph.files: HashMap<String, FileInfo>`.
- `CodeGraph::relationships_for_filtered(id, dir, kinds: &[RelationshipKind], min_conf: f32)`.

- [ ] Write failing tests: unknown kinds map to `Other`; legacy `SqliteStorage` rows fill defaults; `files` derives from nodes; filtered query respects kind + confidence.
- [ ] Implement mapping + accessors; update all callers.
- [ ] Run tests → pass; app opens legacy DB.
- [ ] Commit: `feat(model): extensible kinds, confidence, files map`.

### Task 1.3: `config.rs`

**Files:** Create `src/config.rs`; Test inline.

**Interfaces:** `ProjectConfig` (serde, all optional with defaults); `load(path) -> Result<ProjectConfig, ConfigError>`; `ConfigError { line, col, message }`.

- [ ] Write failing tests: empty `{}` → defaults; unknown fields ignored; bad JSON reports line/col; `codegraph.config.json` in project root is preferred over app dir.
- [ ] Implement serde defaults + `serde_json` error position mapping.
- [ ] Run tests → pass.
- [ ] Commit: `feat(config): project config with defaults and positioned errors`.

### Task 1.4: `settings.rs`

**Files:** Create `src/settings.rs`; Test inline.

**Interfaces:** `Settings { last_project, recent: Vec<PathBuf>, last_legacy_db, edge_kinds, min_confidence, color_by }`; `load_or_default()`, `save()`.

- [ ] Write failing tests: round trip; corrupt JSON → defaults, no panic.
- [ ] Implement.
- [ ] Commit: `feat(settings): persist app settings with corrupt-file fallback`.

### Task 1.5: Stable ids

**Files:** Create `src/indexer/ids.rs` (or `src/ids.rs`); Test inline.

**Interfaces:** `pub fn symbol_id(rel_path, kind, qualified_name, ordinal) -> SymbolId` = first 16 hex of `blake3("{path}\0{kind}\0{qname}\0{ordinal}")`.

- [ ] Write failing tests: deterministic; different ordinal disambiguates; stable across re-index.
- [ ] Implement.
- [ ] Commit: `feat(ids): stable content-addressed symbol ids`.

**Gate:** validation green; legacy DB renders as before.

---

## Phase 2 — Indexer core + TypeScript Tier 1

### Task 2.1: `walk.rs`

**Files:** Create `src/indexer/walk.rs`; Test inline with temp dirs.

**Interfaces:** `walk_files(root, opts) -> (Vec<PathBuf>, Vec<Skip>)`; respects `.gitignore`, default ignores, 1 MB cap.

- [ ] Failing tests: ignores `node_modules`, `target`, `.git`, `dist`; skips >1 MB with reason; includes nested source files.
- [ ] Implement with `ignore::WalkBuilder`.
- [ ] Commit: `feat(indexer): file discovery with ignore rules`.

### Task 2.2: `facts.rs`

**Files:** Create `src/indexer/facts.rs`.

**Interfaces:** `FileFacts { symbols: Vec<RawSymbol>, imports, refs, metrics }`, `RawSymbol`, `RawImport`, `RawRef`, `RawMetrics` (serde).

- [ ] Add serde derives + tests for round trip.
- [ ] Commit: `feat(indexer): fact types`.

### Task 2.3: `cache.rs`

**Files:** Create `src/indexer/cache.rs`; Test with temp SQLite.

**Interfaces:** schema from §5.6; `open`, `get_meta`, `set_meta`, `upsert_file`, `load_facts`, `replace_graph`, `load_graph`; `schema_version`/`extractor_version` mismatch → rebuild.

- [ ] Failing tests: create/open; round-trip facts; version mismatch forces rebuild; replace_graph is transactional.
- [ ] Implement.
- [ ] Commit: `feat(indexer): sqlite cache with schema versioning`.

### Task 2.4: `indexer/mod.rs` pipeline

**Files:** Create `src/indexer/mod.rs`.

**Interfaces:** `index_project(root, opts, progress: Sender<IndexProgress>, cancel: Arc<AtomicBool>) -> Result<IndexSummary, IndexError>`; stages Walk→Hash→Parse→Resolve→Write→Load.

- [ ] Failing test: index a temp 3-file project; assert node/edge counts; cancel flag stops early and leaves cache valid.
- [ ] Implement orchestration, rayon parse, `catch_unwind`, batched writes, single-transaction graph replace.
- [ ] Commit: `feat(indexer): pipeline with progress, cancel, atomic swap`.

### Task 2.5: TypeScript extractor

**Files:** Create `src/indexer/lang/mod.rs`, `src/indexer/lang/typescript.rs`; Test inline + proptest.

**Interfaces:** `LanguageExtractor` trait per spec; registry `for_extension(ext) -> Option<&dyn LanguageExtractor>`.

- [ ] Failing tests on snippets: functions, arrow-const, class+methods, interface, enum, type alias, exports/default, re-exports.
- [ ] Failing proptest: never panics on arbitrary bytes/UTF-8.
- [ ] Implement with tree-sitter (TSX grammar for `.tsx/.jsx/.js/.mjs/.cjs`, TS for `.ts`); `catch_unwind` at call site.
- [ ] Commit: `feat(lang): typescript extractor`.

### Task 2.6: Tier 1 resolution + tsconfig

**Files:** Create `src/indexer/resolve.rs`; `tests/fixtures/ts_app/` + `expected_edges.json`.

**Interfaces:** `resolve_tier1(facts_by_file, config) -> Vec<Edge>`; tsconfig comment/trailing-comma stripping; extension probing per Appendix D.

- [ ] Create fixture (≈12 files) with barrel, `@/`, tsconfig paths, class extends/implements, external package, one cycle, one low-cohesion file.
- [ ] Failing test asserting exact `(source,target,kind)` edges.
- [ ] Implement; external specifiers counted, not edged.
- [ ] Commit: `feat(resolve): typescript tier-1 imports and tsconfig paths`.

### Task 2.7: `IndexStorage` + UI integration

**Files:** Create `src/storage.rs::IndexStorage`; modify `main.rs`.

**Interfaces:** `IndexStorage: Storage` over the cache; UI `Open Project` folder dialog, progress bar, Cancel, summary line, recent projects, auto-load last cache then background refresh.

- [ ] Failing test: incremental equivalence — index, modify one file, incremental result == fresh full index (node+edge sets).
- [ ] Implement UI wiring with worker thread + channel.
- [ ] Commit: `feat(ui): open project, progress, cancel, incremental refresh`.

**Gate:** TS fixture indexes end-to-end from the UI; validation green.

---

## Phase 3 — Tier 2, Python, Rust

### Task 3.1: Tier 2 resolution (TS)

**Files:** Modify `src/indexer/resolve.rs`; Test inline.

**Interfaces:** order local → import-binding (re-export depth 5) → unique-name (0.4) → method-name (0.3) → unresolved; attaches `confidence`/`provenance`.

- [ ] Failing tests: local beats import beats unique; ambiguous stays unresolved; re-export chain resolves; confidence in 0..=1 (proptest: no edge to missing node).
- [ ] Implement.
- [ ] Commit: `feat(resolve): tier-2 symbol resolution with confidence`.

### Task 3.2: Python extractor + resolution

**Files:** `src/indexer/lang/python.rs`; fixture `tests/fixtures/py_pkg`.

- [ ] Failing tests: package `__init__`, relative imports, alias calls, Protocol + impl, cycle.
- [ ] Implement; `src/` layout detection.
- [ ] Commit: `feat(lang): python extractor and resolution`.

### Task 3.3: Rust extractor + workspace resolution

**Files:** `src/indexer/lang/rust_lang.rs`; fixture `tests/fixtures/rust_ws`.

- [ ] Failing tests: `mod` both layouts; `use crate/self/super`; `impl Trait for Type`; cross-member use from workspace `Cargo.toml`.
- [ ] Implement.
- [ ] Commit: `feat(lang): rust extractor and workspace resolution`.

### Task 3.4: Per-function metrics (all languages)

**Files:** Modify extractors + `facts.rs`.

- [ ] Failing tests: cyclomatic counts per Appendix A.4 (one test per language construct); nesting/params/LOC.
- [ ] Implement.
- [ ] Commit: `feat(metrics): per-function complexity`.

### Task 3.5: Self-hosting test

**Files:** `tests/self_host.rs`.

- [ ] Test: index this repo; assert `main.rs` depends on lib modules, `analysis.rs` depends on `graph.rs`, no panics, resolved ratio above a floor derived from the real number.
- [ ] Commit: `test: self-hosting index smoke`.

**Gate:** three languages index fixtures; self-hosting passes; validation green.

---

## Phase 4 — Metrics rewrite

### Task 4.1: `metrics.rs` (coupling/cohesion/abstractness)

**Files:** Create `src/metrics.rs`; `docs/METRICS.md`.

- [ ] Failing tests: hand-computed Ca/Ce/I/CBO; cohesion components/largest_ratio/internal_ratio; A/D; `ca_edges`/`ce_edges` retained; contains excluded; weight/min_confidence effects.
- [ ] Implement exactly Appendix A.1–A.3; delegate old `coupling_report`.
- [ ] Commit: `feat(metrics): coupling, cohesion, abstractness, main sequence`.

### Task 4.2: Complexity roll-ups

**Files:** Modify `src/metrics.rs`.

- [ ] Tests: file-level sum/max/fn count/LOC.
- [ ] Implement.
- [ ] Commit: `feat(metrics): complexity roll-ups`.

### Task 4.3: UI audit panel

**Files:** Modify `main.rs`.

- [ ] Implement new columns + `?` help + index-quality indicator; tests for pure formatting helpers.
- [ ] Commit: `feat(ui): metric table with help and quality`.

**Gate:** validation green; `docs/METRICS.md` matches code.

---

## Phase 5 — Modules, warnings, baselines

### Task 5.1: `modules.rs`

**Files:** Create `src/modules.rs`; `docs` config examples.

- [ ] Failing tests: first-match wins; overlap reported; `app/(admin)/**` literal; Unassigned bucket; `allowed_deps` violation; derived fallback when no modules.
- [ ] Implement with `globset`; `build_module_overview_with(config)`.
- [ ] Commit: `feat(modules): glob assignment and overview`.

### Task 5.2: `warnings.rs` (Appendix B)

**Files:** Create `src/warnings.rs`.

- [ ] Failing test per rule (fires + does not fire); stable `id` across re-index; explanation/suggestion under three sentences.
- [ ] Implement all rules with config thresholds.
- [ ] Commit: `feat(warnings): rule engine`.

### Task 5.3: `analysis::analyze` bundle

**Files:** Modify `src/analysis.rs`.

- [ ] Test: bundle contains metrics/cycles/structure/warnings; computed on worker (no UI types).
- [ ] Implement `AnalysisBundle` + `analyze()`.
- [ ] Commit: `feat(analysis): single-pass bundle`.

### Task 5.4: `baseline.rs`

**Files:** Create `src/baseline.rs`.

- [ ] Tests: save/load; diff new/resolved/unchanged; ids stable.
- [ ] Implement.
- [ ] Commit: `feat(baseline): snapshots and diffs`.

### Task 5.5: Module editor + warnings panel

**Files:** Modify `main.rs`.

- [ ] Implement JSON editor window (validate/save/reload/preview/overlap/error line-col); warnings tab (severity text+icon+color, filters, explain, set baseline, header badge); click-to-focus + highlight set passed to draw.
- [ ] Commit: `feat(ui): modules and warnings panels`.

**Gate:** warnings on a real project clickable/filterable/baseline-diffable; validation green.

---

## Phase 6 — Structural troubleshooting

### Task 6.1: `impact.rs`

- [ ] Proptest: reverse closure == brute force; depth/node caps honored.
- [ ] Implement forward/reverse closure over chosen edge kinds.
- [ ] Commit: `feat(impact): transitive closure`.

### Task 6.2: Path finder

- [ ] Test: returns valid shortest path; empty when disconnected.
- [ ] Implement BFS edge-kind filtered.
- [ ] Commit: `feat(impact): shortest dependency path`.

### Task 6.3: `structure.rs`

- [ ] Proptest: feedback arc set makes random digraphs acyclic, empty for DAGs; articulation points match brute force; reachability from entry points.
- [ ] Implement Eades-Lin-Smyth, iterative Tarjan, Brandes (sampled >3000).
- [ ] Commit: `feat(structure): cycle breaking, articulation, betweenness, reachability`.

### Task 6.4: `dsm.rs` + Matrix view

- [ ] Tests: condensation order places non-SCC cells one side; weight totals equal aggregated edges; bounded sizes.
- [ ] Implement builder + pure matrix draw.
- [ ] Commit: `feat(dsm): dependency structure matrix`.

### Task 6.5: UI tabs (Impact/Path/Cycles/Structure/Matrix)

- [ ] Implement; commit: `feat(ui): structural analysis tabs`.

**Gate:** each tool works from the UI on a real project; validation green.

---

## Phase 7 — Git analytics

### Task 7.1: `git.rs` parser

- [ ] Tests on canned `git log` (empty, binary `-`, merges); Windows `CREATE_NO_WINDOW`; graceful disable.
- [ ] Implement via `std::process::Command`.
- [ ] Commit: `feat(git): churn parser`.

### Task 7.2: Hotspots (A.5)

- [ ] Tests on synthetic data; commit: `feat(git): hotspot ranking`.

### Task 7.3: Co-change (A.6) + hidden coupling

- [ ] Tests: cutoff >30 files; support/confidence thresholds; static-path check.
- [ ] Commit: `feat(git): logical coupling`.

### Task 7.4: Cache by HEAD + UI tabs

- [ ] Tests: unchanged HEAD skips re-run; commit + UI.

**Gate:** hotspots + hidden coupling show for a real repo; validation green.

---

## Phase 8 — Polish, persistence, export, CLI

### Task 8.1: Edge filter + min confidence

- [ ] Tests: filter changes built view; commit.

### Task 8.2: Tooltip, fit-to-view, reset, keys

- [ ] Tests: hit-test helper; commit.

### Task 8.3: Color-by + palette

- [ ] Tests: color mapping pure; commit.

### Task 8.4: Persistence

- [ ] Tests: settings round trip; commit.

### Task 8.5: `report.rs` JSON/Markdown

- [ ] Test: byte-identical output on repeated runs; commit.

### Task 8.6: CLI `cg_index`

**Files:** `src/bin/cg_index.rs`.

- [ ] Tests: fixture exit 0; baseline exit 2; `--no-git`; commit.

**Gate:** CLI works on a fixture and this repo; validation green.

---

## Phase 9 — Hardening & docs

### Task 9.1: Performance + ignored synthetic test

- [ ] Add `#[ignore]` test generating ~1500 files; assert spec targets; run `cargo test -- --ignored`.
- [ ] Commit.

### Task 9.2: Docs + cleanup

- [ ] Finalize `README.md`, `ARCHITECTURE.md`, `docs/METRICS.md`, `PROGRESS.md`; remove stale iced remnants; commit.

**Gate:** all global acceptance criteria met.

---

## Execution Handoff

Plan saved to `docs/superpowers/plans/2026-11-11-codegraph-self-indexing.md`.

Choose execution method:
- **Subagent-driven** — a fresh subagent implements each task, a fresh reviewer checks it, then a whole-branch review. Most thorough; recommended here because the tasks are interface-heavy and a shipped mistake (wrong resolution edge, broken cache) is expensive.
- **Native** — implement in-session, one fresh review at the end. Faster; relies on the plan carrying the design.

Recommended: **Subagent-driven**, because Phases 2–3 (extractors/resolution) have many exact-interface contracts and benefit from independent review per task.
