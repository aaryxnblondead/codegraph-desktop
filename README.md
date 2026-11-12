# codegraph_desktop

A desktop **CodeGraph knowledge-graph explorer** written in Rust 2021, following the [rust-engineer](https://jeffallan.github.io/claude-skills/skills/language/rust-engineer/) guidelines:

- Ownership & borrowing for memory safety; owned data flows across API boundaries (no dangling borrows from SQLite).
- Trait-based design: `Storage` with an associated `Error` type; `SymbolId` branded newtype.
- Idiomatic, `unsafe`-free code; every error explicit via [`thiserror`] and `?` — no `unwrap()` in production paths.
- `cargo clippy --all-targets --all-features`, `cargo fmt --check`, and `cargo test` all clean; `cargo build --release` produces a native `.exe`.

## What it is

A desktop GUI (native window, no browser) that **indexes a project folder itself** and helps you find design problems:

- **Open Project…** — pick a folder; the app walks it (respecting `.gitignore`), parses TypeScript/JavaScript with tree-sitter, resolves imports/references, and builds the graph. **No `codegraph.db` required.**
- **Open database…** — still opens a legacy `codegraph.db` (tables `nodes`/`edges`).
- **Whole graph overview** — files become super-nodes with aggregated dependency **arrows**; hierarchical layered layout (dependencies flow top → bottom).
- **Drill-down** — click a module → its files; click a file → its hottest symbol's ego-network; click a symbol to refocus.
- **Troubleshooting tabs (bottom panel):**
  - **Audit** — cycles + most-unstable files (Ca/Ce/instability/cohesion) + **Set baseline**.
  - **Warnings** — rule engine (cycles, high instability, low cohesion, god files, layer violations, fan-out, complex functions, main sequence, orphans) with severity, suggestion, and click-to-focus.
  - **Impact** — transitive dependents/dependencies by depth for the selected symbol.
  - **Hotspots** — git churn ranking (commits, added/deleted).
  - **Hidden coupling** — files that change together with no direct static edge.
- **Config** — `codegraph.config.json` (or the app config dir) defines modules/globs, allowed dependencies, ignore globs, and thresholds.
- **Theme:** tactical telemetry / industrial-brutalist — dark substrate, phosphor text, hazard-red accents, monospace, zero border-radius.

## Scale & performance

- **Self-indexing (release)** of the MyPM `mypm-phase1` project: **867 files → 20,136 symbols / 43,225 edges in 11.1 s** (no `codegraph.db`).
- **Async** — indexing and git analytics run on worker threads; the window stays responsive; indexing is cancellable.
- **Bounded rendering** — module overview ≤120 super-nodes; file view ≤160; symbol ego ≤80. The full symbol graph is never drawn at once.
- **Precomputed layout** — positions computed once, always finite; pure draw path.

## Data source

The app starts with a small embedded **demo graph**. Use **Open Project…** to index a folder, or **Open database…** for a legacy `codegraph.db`.

Schema used by `SqliteStorage` (matches the real CodeGraph database):

```sql
-- nodes: code symbols
SELECT id, name, kind, file_path, start_line, signature, docstring FROM nodes;

-- edges: relationships between symbols (source/target are node ids)
SELECT id, source, target, kind FROM edges;
```

`nodes.id`, `edges.source`, and `edges.target` are TEXT (UUIDs/hashes), which is
why `SymbolId` wraps a `String`.

## Build

```bash
# type-check (no linker needed for this crate's own code; dependency
# build scripts still need a linker — see Environment note below)
cargo check

# lints (rust-engineer requirement: fix every warning)
cargo clippy --all-targets --all-features

# formatting
cargo fmt --check

# tests (unit + property-based via proptest)
cargo test

# release build
cargo build --release
```

SQLite is enabled by default (`default = ["sqlite"]`), so `cargo run` opens with database support. To build without SQLite (demo-only, no C compiler needed):

```bash
cargo run --no-default-features
```

## Build requirements

A working MSVC toolchain (`link.exe`/`cl.exe` + Windows SDK libs) is required to link the executable. Install the **"Desktop development with C++"** workload via the Visual Studio Installer, then run the commands above.

A project `.cargo/config.toml` points rustc at the MSVC linker and the Windows SDK lib paths so `kernel32.lib`/`user32.lib`/`ws2_32.lib` resolve. A release build (`cargo build --release`) produces `target/release/codegraph_desktop.exe`, a native desktop window (no browser/Electron).

## Project layout

```
src/
  lib.rs      — crate root; re-exports model, error, graph, storage
  model.rs    — Symbol, SymbolId, SymbolKind, Relationship, RelationshipKind, EdgeDirection
  error.rs    — CodegraphError (thiserror; Database/SymbolNotFound/DuplicateSymbolId/Schema/Io)
  graph.rs    — CodeGraph: in-memory graph with id/outgoing/incoming indexes
  storage.rs  — Storage trait + MemoryStorage (default) + SqliteStorage (feature sqlite)
  main.rs     — egui/eframe desktop app + custom graph painter
```

## Rust-engineer compliance checklist

- [x] Ownership/borrowing — data owned (`Vec<Symbol>`, `String`); no connection borrows leak.
- [x] Trait design — `Storage` trait + associated `Error`; branded `SymbolId` newtype.
- [x] Safe implementation — zero `unsafe`; idiomatic Rust 2021.
- [x] Error handling — `thiserror` enum, `#[from]` propagation, `?` operator, no `unwrap()` in prod.
- [x] Validation — `cargo clippy --all-targets --all-features`, `cargo fmt --check`, `cargo test` all clean; `cargo build --release` produces a native `.exe`.
- [x] Tests — unit tests in `model.rs`/`graph.rs`/`storage.rs`; property-based tests via `proptest` dev-dependency.
- [x] Documentation — module-level docs, design notes, schema assumptions.

## Headless CLI (CI ratchet)

`cg_index` indexes a folder and writes reports without the GUI:

```bash
cg_index <project_root> [--json out.json] [--markdown out.md]
         [--baseline baseline.json] [--fail-on-new error|warn]
         [--no-git] [--min-confidence 0.5]
```

Exit codes: `0` ok, `1` failure, `2` new warnings at or above
`--fail-on-new` compared with `--baseline`. Baselines are written by the app
("Set baseline") or by any JSON file with `{"created","warnings","module_metrics"}`.

Example on this repo: `34 files · 604 symbols · 1653 edges · 19% refs resolved`,
then `warnings: 3 error · 48 warn · 6 info`.

## Index cache

Each project gets `…/projects/<id>/index.sqlite` under the app cache directory
(never inside the project). Files are re-parsed only when their content hash
changes, so the second index of a large project is much faster than the first
(measured: 20.3 s cold → 1.24 s warm on 867 files). The app shows the cached
graph immediately at startup and refreshes in the background.

## Interface

A dark instrument-panel layout: header, symbol list, canvas, and a fixed tab
strip at the bottom.

- **Header** — project path, index status (`indexed 868 files · 20,156 symbols ·
  43,332 edges · 35% refs resolved · 736 ms`), and the primary actions.
  `Ctrl+O` opens a project. `Esc` clears the selection.
- **Symbols (left)** — searchable rows with a language/kind badge, the name, and
  the `file:line`. Arrow keys walk the list, `Enter` opens its impact.
- **Canvas** — dot-grid surface, nodes coloured by kind, `drag` to pan, `scroll`
  to zoom, `− / 100% / + / fit` controls, and a hover tooltip with the node's
  kind, file, and in/out degree. Labels avoid colliding with each other.
- **Breadcrumb** — `Overview / module / file / symbol`, each level clickable.
- **Tabs** — Overview (stat tiles, most-coupled table, next steps), Modules
  (Ca/Ce/I/A/D/cohesion table with column tooltips), Warnings (severity filters,
  search, expandable "why / what to try"), Cycles (groups with the cheapest
  edges to cut), Impact, Hotspots, Hidden coupling.
- **Modules editor** — validate/save JSON, generate a starter config from
  folders, and see a live preview including files matched by more than one
  module.

### Accessibility

- All text pairs meet WCAG AA contrast on both panel and card surfaces
  (measured: body 14.8:1, secondary 7.0:1, muted 5.8:1, accent 5.6:1, error
  5.5:1, warning 8.8:1, success 8.4:1).
- Severity and state never rely on colour alone: chips carry the word
  (`ERROR`, `WARN`, `INFO`) and counts; the selected tab is a filled pill, not
  just a colour change.
- Rows are 42 px tall with a whole-row click target and a visible hover/selected
  surface; buttons are padded to ~30 px.
- Keyboard: `Ctrl+O` open, `↑/↓` move through symbols, `Enter` opens impact,
  `Esc` clears selection.
- Screen readers: every custom-painted control carries a name and role through
  AccessKit. Symbol rows are announced as `"fn parse_input — src/parser.rs:12"`
  buttons with their selected state; graph nodes as
  `"app/admin — app/admin"` buttons; column headers include their definition
  (`"Ce, column — Efferent coupling: how many other units this one depends on."`).
  `tests/accessibility.rs` renders the real widgets and asserts the tree, roles,
  focus and click activation.
- Metric column headers explain themselves on hover (`Ca`, `Ce`, `I`, `A`, `D`,
  cohesion, complexity).

## Live sync

The app watches the project folder and keeps the graph current while you edit:

- **Touched files light up.** A save marks the file (and the module that contains
  it) with a warm amber dot in the list, a ring plus glow in the graph, a `●`
  marker in the Modules table, and a `● N touched` chip in the header. The
  highlights fade after 20 seconds.
- **Incremental re-index, automatically.** Edits are debounced for 600 ms, then
  only changed files are re-parsed (content-hash cache) and the graph is swapped
  in. Measured on a 5-file project: **29 ms** from save to updated graph.
- **Watching is a toggle** in the header (`watching` / `watch off`), and it never
  writes inside the project. `.git` is ignored; branch moves are handled by the
  git layer instead.

## Git branches and worktrees

- The header shows the current branch, the HEAD short hash, and
  `worktrees N · branches M`.
- The dropdown lists every worktree with its branch and path, plus the local
  branches (most recent first). Choosing a worktree switches the indexed root to
  that folder and indexes it.
- If HEAD changes (a checkout or a commit), the graph re-indexes automatically.
- Git is read-only: the app never checks out, commits, or pushes.

## Copy anything

Every analysis pane has a copy action that puts plain text on the clipboard:

| Where | Action | Payload |
|---|---|---|
| Overview | `copy summary` | project @ branch (sha) · files / symbols / edges |
| Overview, Warnings | `copy warnings` / `copy all` | `[ERROR] rule — message` + `→ suggestion` per finding, with counts header |
| Warnings | `copy` (per card) | that single finding |
| Cycles | `copy` (per group) | file list + suggested cuts with weights |
| Impact | `copy impact` | dependents/dependencies counts and depth breakdown |
| Modules | `copy table` | one line per module with Ca/Ce/I/A/D/cohesion/cx |
| Detail | `copy ref` | `path:line  name` |

The formatting lives in `report.rs` and is unit-tested, so pasted text is stable;
`tests/clipboard.rs` clicks the buttons through the accessibility tree and
asserts the actual `CopyText` payload.

## Findings you can act on

Every finding names its target, not just its file: `src/warnings.ts::run_warnings_reporting:137–610`.
The rules are calibrated against real reports:

- `complex_function` is symbol-level. Branches inside JSX markup are excluded,
  render functions are capped at WARN, and ERROR means cyclomatic ≥ 45.
- `god_file` requires a size floor (300 LOC / 40 symbols) before degree counts,
  so a 22-line shared type with heavy fan-in is not flagged.
- `low_cohesion` requires ≥ 15 symbols, ≥ 4 disconnected groups and no dominant
  group; at most 10 files are listed plus one volume note.
- Unresolved references are reported as `index_quality` (an index number), never
  as dead code.

### Deltas, not haystacks

With a baseline, the summary leads with what changed:

```text
findings: 3 error · 29 warn · 9 info  (1 new, 0 fixed; errors +1, warnings +0)
NEW:
  ERROR complex_function   `src/warnings.rs::run_warnings_reporting:137–610` …
FIXED:
  WARN  low_cohesion       `src/legacy/report.ts` …
by rule: low_cohesion 11, complex_function 9, …
```

`cg_index --save-baseline baseline.json` accepts today's findings as debt;
`--baseline baseline.json` reports new vs fixed on every later run, and
`--fail-on-new error|warn` exits 2 only for regressions. The desktop app shows
the same block under *Changed since baseline*.

### Suppression that lives in the repo

```json
{ "suppress": [
  { "rule": "complex_function", "path": "app/legacy/**", "reason": "vendor port" }
] }
```

```ts
// codegraph-ignore complex_function
function paymentWebhookSwitch(payload: Webhook) { … }
```

Inline comments apply to the function they sit on (within a few lines); a bare
`// codegraph-ignore` covers the file. Suppressed findings are reported as one
`suppressed` note, so the decision stays visible.
