# Decisions

| Date | Decision | Reason |
|---|---|---|
| 2026-11-11 | Parser stack: `tree-sitter = 0.25`, `tree-sitter-typescript = 0.23`, `tree-sitter-python = 0.25`, `tree-sitter-rust = 0.24`. | Mutually compatible; verified with `cargo check`/release build. |
| 2026-11-11 | Use the TSX grammar for `.tsx/.jsx/.js/.mjs/.cjs`, plain TS grammar for `.ts`. | JSX requires the TSX grammar; `.ts` stays on the stricter grammar. |
| 2026-11-11 | Create a synthetic file-level `Module` symbol per file; imports are file→file edges. | The graph is symbol-based, but coupling/cohesion need file/module units. Gives a clean file layer without a second graph type. |
| 2026-11-11 | Unknown kind strings map to `Other(String)` (kinds are non-`Copy`). | Legacy DBs and future languages must never fail to load. Verified by `tests/legacy_db.rs`. |
| 2026-11-11 | Tier 2 resolution order: local → import-binding → unique-name → method-name. | Conservative; avoids wrong edges. Ambiguous names stay unresolved. |
| 2026-11-11 | `method-name` resolution is 0.3 confidence and only when the method name is globally unique. | Below the default `min_confidence` (0.5), so methods on unknown receivers never drive metrics — they only fill in the raw graph. |
| 2026-11-11 | Metrics formulas are documented heuristics. | The GFG article defines concepts, not formulas; see `docs/METRICS.md`. |
| 2026-11-11 | Git via the `git` CLI, not `git2`. | Avoids another C build; Windows console suppressed with `CREATE_NO_WINDOW`. |
| 2026-11-11 | `rusqlite` is a normal (non-optional) dependency; the `sqlite` feature is empty. | The index cache needs SQLite anyway, so a build without SQLite has no use. Legacy DB support is always available. |
| 2026-11-11 | Index cache is per project under the app cache dir (`projects/<id>/index.sqlite`), never inside the project. | The app must never write inside a project it indexes. |
| 2026-11-11 | Cache facts are keyed by blake3 of file content; `EXTRACTOR_VERSION` bump clears facts. | Content hashing avoids mtime clocks; a version bump forces a clean re-parse after extractor changes (currently `"2"`). |
| 2026-11-11 | Python: `from . import x` is treated as importing the submodule `x`, not the package. | That is what Python imports; resolving to `__init__.py` would create the wrong edge. |
| 2026-11-11 | Python absolute imports resolve from the project root and `src/`. | Covers flat and src-layout projects without asking for config. |
| 2026-11-11 | Rust `use` paths resolve through a module map built from `src/…` file layout; package-name paths (`my_crate::m`) map to the crate root. | `crate::`/`self::`/`super::` alone would miss binaries referring to their own library. |
| 2026-11-11 | Cycle breaking uses a greedy Eades–Lin–Smyth ordering, not an exact minimum feedback arc set. | Minimum FAS is NP-hard; the greedy set is cheap, deterministic, and verified acyclic before it is shown. |
| 2026-11-11 | Reports sort modules, warnings, cycles and files before writing. | Byte-identical output on the same input, so reports diff cleanly and can be committed. |
| 2026-11-11 | `cg_index` exits 2 only for *new* warnings at/above `--fail-on-new` versus a baseline. | Turns the tool into an opt-in CI ratchet without breaking existing projects. |
| 2026-11-12 | The palette and custom-painted widgets live in the library (`src/ui.rs`); every other lib module stays egui-free. | The accessibility tests must render the *real* widgets, and integration tests cannot import from a binary crate. `ui` is the single documented GUI exception; `graph`, `indexer`, `metrics`, `analysis`, `warnings` etc. remain headless. |
| 2026-11-12 | Every painter-drawn control publishes `Response::widget_info(...)` with a label and role. | Painted text is invisible to AccessKit; without widget info a screen reader announces an unlabelled clickable rectangle. Verified by `tests/accessibility.rs`. |
| 2026-11-12 | Graph nodes are individual `ui.interact` widgets (>= 24 px targets) instead of a manual hit test. | Gives each node a real AccessKit node with a label, focus and activation, and makes the hit target large enough for motor-impaired users. |
| 2026-11-12 | `egui_kittest` is a dev-dependency. | It renders the UI and exposes the AccessKit tree, so accessibility is tested automatically instead of claimed. |
| 2026-11-12 | `notify` 8.2 for file watching (default on), 600 ms debounce before re-index. | True push events instead of polling; the debounce collapses a burst of editor saves into one incremental index. |
| 2026-11-12 | Touched files are highlighted for 20 s and matched by prefix for modules. | Long enough to notice a save, short enough that the canvas settles; prefix matching lights the module that contains the file. |
| 2026-11-12 | Git stays read-only: branch/worktree state is read, a worktree can be opened, nothing is checked out or committed. | Switching branches would mutate the user's working tree; the app only observes and indexes. |
| 2026-11-12 | Copy payload formatting lives in `report.rs`, not in the UI. | Stable, unit-tested text for issues and chat; the UI only calls `ctx.copy_text`. |
| 2026-11-12 | Findings carry `file::symbol:line–end`, and `complex_function` is computed per symbol with JSX branches excluded. | File-level messages were unactionable; JSX conditionals inflated render functions into false errors. |
| 2026-11-12 | `god_file` needs a 300 LOC / 40 symbol floor before degree counts; `low_cohesion` needs 15+ symbols, 4+ groups, no dominant group, and lists at most 10. | A 22-line type with 572 inbound refs is healthy; a rule that fires on a third of the codebase measures its own threshold. |
| 2026-11-12 | Severity thresholds: ERROR at cyclomatic ≥ 45 (never for render functions), WARN at ≥ 20. | ERROR must mean "this will probably break", or the tier stops carrying information. |
| 2026-11-12 | Suppression is committed config (`suppress`) plus inline `codegraph-ignore`. | Accepted debt should be reviewable and travel with the code, not live in a machine-local baseline. |
| 2026-11-12 | The CLI summary is budgeted (counts → delta → 10 new / 5 fixed → rule histogram → 10 findings); `--json` carries everything. | 990 findings of prose cost more context than they return; the array is for tools, the summary for humans. |
