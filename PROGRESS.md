# CodeGraph Desktop — Progress

Status: `todo` | `partial` | `done`.

- [x] Phase 0 — Recon & baseline
- [x] Phase 1 — Foundations (model, paths, config, settings, ids)
- [x] Phase 2 — Indexer core + TypeScript Tier 1 (walk, extract, resolve, **SQLite cache**, incremental reuse, `IndexStorage`, startup cache load)
- [x] Phase 3 — Tier 2, Python, Rust (all three extractors + resolution; self-hosting test)
- [x] Phase 4 — Metrics rewrite (coupling/cohesion/abstractness/complexity + docs)
- [x] Phase 5 — Modules, warnings, baselines (engine + Modules tab + JSON module editor with validation, live preview, overlap report, starter generation)
- [~] Phase 6 — Structural troubleshooting (impact, path finder, cycle-breaking cuts + Cycles tab, DSM builder; matrix UI + articulation/betweenness not built)
- [x] Phase 7 — Git analytics (churn, hotspots, co-change, hidden coupling + UI)
- [~] Phase 8 — Polish, persistence, export, CLI (report export + `cg_index` done; edge filters, tooltips, color-by not built)
- [~] Phase 9 — Hardening & docs (METRICS/README/ARCHITECTURE/PROGRESS written; UI overhaul + AccessKit-verified accessibility done; large synthetic perf test not built)

## Baseline (Phase 0)

- `cargo test --all-features`: 22 passed at start; currently **112 passed**.
- `cargo clippy --all-targets --all-features`: 0 warnings.
- `cargo fmt --check`: clean.
- `cargo build --release`: succeeds (GUI + `cg_index`).
- Real data point (legacy DB): 12,732 symbols, 35,812 relationships, 876 files, 29 cycle groups.

## Evidence (real runs)

- Self-index of `C:\Users\myind\Documents\GitHub\MyPM\MypmTech\mypm-phase1`
  (867 TS/JS files, **no `codegraph.db`**): 20,136 symbols / 43,225 edges,
  35% refs resolved.
  - cold index: 20.3 s
  - warm index (fact cache): **1.24 s**, 867 cached / 0 parsed
  - `load_cached_graph`: **193 ms** — the UI shows the graph immediately at startup
- Self-index of this Rust repo through the CLI:
  `34 files · 604 symbols · 1653 edges · 19% refs resolved`.
  - cold: 634 ms; warm: **162 ms**.
- Release binaries: `target\release\codegraph_desktop.exe` (GUI) and
  `target\release\cg_index.exe` (CLI).

## Phase log

### Phase 0
Recorded baseline (22 tests), wrote `ARCHITECTURE.md`, `DECISIONS.md`, this file.

### Phase 1
- Extensible `SymbolKind`/`RelationshipKind` with `Other(String)` and `parse`.
- `Symbol` gained `end_line`, `exported`, `qualified_name`, `lang`.
- `Relationship` gained `line`, `confidence`, `provenance`.
- `FileInfo` + `CodeGraph.files` + `relationships_for_filtered`.
- `paths`, `config` (line/col errors), `settings`, `ids` (stable blake3 ids).

### Phase 2
- `indexer::{walk, facts, lang, resolve, cache, mod}`; tree-sitter TS/TSX extractor.
- Tier 1 imports + tsconfig-ish aliases (`@/`, `src/`); Tier 2
  (local → import-binding → unique-name → method-name).
- **On-disk cache** (`indexer/cache.rs`): per-file facts keyed by blake3 content
  hash, `meta` schema/extractor versions (mismatch clears silently), graph
  tables replaced in one transaction, stale files pruned.
- **Incremental pipeline**: `index_project_cached` reuses cached facts for
  unchanged files and only re-parses changes.
- **`IndexStorage`** loads the cached graph at open; the app adopts it before
  indexing starts, then refreshes in the background.
- UI: "Open Project…", progress, Cancel, last-project restore from settings.

### Phase 3
- **Python** extractor: functions, classes, methods (`Class::method`),
  `import`/`from … import` (dotted + relative), calls, cyclomatic complexity.
- **Rust** extractor: fn/struct/enum/trait/impl/mod/const/static/type, `impl T
  for S` (implements), `Type::method` qualification, `use` trees (nested,
  aliases, wildcards), calls, complexity.
- Resolution: Python dots/`__init__.py`/`src` layout; Rust `crate::`/`self::`/
  `super::`, module `foo.rs` vs `foo/mod.rs`, package-name paths for bins.
- `tests/self_host.rs` indexes this repo: Rust symbols, `main.rs` → lib,
  `analysis.rs` → `graph.rs`, no parse failures, resolution floor + quality note.

### Phase 4
- `metrics.rs`: Ca/Ce/CBO/instability, abstractness/distance, cohesion
  components/largest_ratio/internal_ratio, complexity rollups.
- `docs/METRICS.md` written; audit panel shows the new fields.

### Phase 5
- `modules.rs` (globset, first-match, overlaps, Unassigned, derived fallback).
- **Modules tab**: module table (Ca, Ce, I, A, D, cohesion groups/largest share,
  complexity) with click-to-drill-down, unassigned-file count.
- **Module editor window**: JSON editor with Validate / Save & re-analyze /
  Reload from disk, "Generate from folders" starter config, live preview of
  files-per-module and a multi-match warning.
- The app now loads `codegraph.config.json` (or the app-config copy) on open,
  so `modules` and `allowed_deps` drive the GUI, not just the CLI.
- `warnings.rs` rules: cycle (with concrete cut edges), high_instability,
  low_cohesion, god_file, layer_violation, fan_out_module, complex_function,
  main_sequence, orphan.
- `baseline.rs` snapshot/diff; "Set baseline" + "since baseline" in audit panel.
- Warnings tab with severity, focus, suggestion.

### Phase 6
- `impact.rs`: reverse/forward closures (depth/cap) + shortest path; Impact tab.
- `structure.rs`: Eades–Lin–Smyth greedy ordering, backward-edge set, verified
  `break_cycle` (falls back to all internal edges if verification fails).
  Cycle warnings now list the edges to cut, cheapest first.
- **Cycles tab**: each file-level group with its members and the cheapest
  edges to cut (clickable).
- `dsm.rs`: matrix builder + topological ordering. **No Matrix view yet.**
- Pending: articulation points, betweenness, DSM UI.

### Phase 7
- `git.rs`: `git log` parser, churn, co-change; Hotspots and Hidden-coupling tabs.
- Git runs on a worker thread with `CREATE_NO_WINDOW`; failures disable quietly.

### Phase 8
- `report.rs`: deterministic JSON + Markdown export (summary, module table,
  warnings with suggestions, cycles with cuts, index quality).
- `src/bin/cg_index.rs`: headless indexer with `--json`, `--markdown`,
  `--baseline`, `--fail-on-new error|warn`, `--no-git`, `--min-confidence`;
  exit codes 0 / 1 / 2 (CI ratchet). Integration tests cover 0 and 2.
- Pending: edge-kind filter UI, hover tooltips, fit-to-view, color-by,
  module JSON editor.

### Phase 9
- Docs written.
- **UI overhaul**: design tokens + contrast-verified palette, card/stat/chip
  primitives, interactive 42px rows, real grid tables with tooltips, severity
  filters and search, cycle cards with cut edges, breadcrumb navigation,
  dot-grid canvas with kind colours, collision-free labels, zoom controls,
  node tooltips, keyboard shortcuts, and a sample-data banner.
- Verified on the 868-file MyPM project: warm re-index 701–736 ms, 20,156
  symbols, 43,332 edges, 2 real cycle groups (after confidence filtering),
  122 errors / 861 warnings surfaced.
- **Accessibility**: custom-painted rows, chips, column headers and graph nodes
  now publish AccessKit nodes; widgets moved to `src/ui.rs` so the real ones can
  be tested. `tests/accessibility.rs` renders them through `egui_kittest` and
  asserts role, label, selected state, focus and click-through (6 tests).
- Pending: `#[ignore]` synthetic 1500-file performance test.

### Phase 10 — Live sync, git worktrees, copyable analysis
- **`watch.rs`**: recursive `notify` watcher, `.git` filtered, project-relative
  paths, debounced batches. Two tests (write event, .git ignored).
- **Incremental live re-index**: 600 ms debounce, cache-backed, only changed
  files re-parsed. `tests/live.rs` asserts a save is reported, one file is
  re-parsed, one reused, edges grow, and `Touched` matches the file *and* its
  module.
- **Touch highlights**: amber dot in the list (touched rows pinned to the top),
  glow ring on the canvas node, `●` in the module table, `● N touched` chip.
- **Git sync**: `repo_state`, `worktrees`, `branches`, `is_dirty` (read-only CLI).
  Header shows branch + HEAD + counts; the dropdown switches worktrees; HEAD
  changes trigger a re-index. 2 tests (porcelain parser, real temp repo).
- **Copyable analysis**: `warning_text`, `warnings_text`, `cycle_text`,
  `impact_text`, `module_text`, `symbol_ref`, `context_text` in `report.rs`,
  wired to copy buttons on every tab. `tests/clipboard.rs` asserts the emitted
  `CopyText` payloads.
- Fixed: a project with no cache now opens and indexes instead of falling back to
  the demo (first-run bug found while testing live sync).
- Evidence: live demo project — `● 1 touched`, `api/routes.ts` ringed,
  `indexed 5 files · 11 symbols · 13 edges · 29 ms` after a save; MyPM repo —
  `main cbfc575 worktrees 12 · branches 13`.

### Phase 11 — Feedback round: actionable findings
- `Warning` gained `locations: Vec<Location>` (`file::symbol:line–end`). Every
  rule fills it when it knows the symbol; copy text and the UI show it.
- `complex_function` is symbol-level; JSX-internal branches are excluded,
  render functions cap at WARN, and ERROR now means cyclomatic ≥ 45.
- `god_file` requires a size floor before degree counts and names its three
  largest symbols. `low_cohesion` requires 15+ symbols / 4+ groups / no dominant
  group, lists at most 10 and adds a volume note. `orphan` became
  `index_quality`, explicitly "not a dead-code list".
- Baseline deltas: `diff` returns new/fixed details plus error/warn count deltas;
  the CLI summary leads with them, the Overview has a "Changed since baseline"
  card, and `cg_index --save-baseline` writes one.
- Suppression: committed `suppress` rules in config and inline
  `// codegraph-ignore <rule>`; one `suppressed` note reports the total.
- CLI summary is budgeted: counts, delta, new/fixed (10/5), rule histogram, and
  the 10 worst findings — instead of every finding as prose.
- Measured on this repo: warnings dropped 48 → 29 after calibration while the
  real `complex_function` at 50 still surfaces as ERROR with its symbol and line
  range. Delta proof: rerun = `0 new, 0 fixed`; removing one baseline entry =
  `1 new, errors +1`.
