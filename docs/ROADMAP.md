# Roadmap

Short, honest, and ordered by what unblocks the most people. Items marked
`good first issue` are scoped so they can land in one sitting — each lists the
files to touch and a way to verify the result.

## Now

- **Dependency Structure Matrix view** — `src/dsm.rs` already builds the matrix;
  it needs a bounded, pure draw path and a header toggle. `src/main.rs`,
  `src/ui.rs`.
- **`cg_index --watch`** — reuse `src/watch.rs` to re-index on change and print
  the delta each time instead of the full list. `src/bin/cg_index.rs`.
- **Light theme** — a second palette in `src/ui.rs` with a theme switch; every
  text pair must still meet 4.5:1. Add contrast numbers to `README.md`.

## Next

- **Articulation points and betweenness** in `src/structure.rs`, surfaced as
  structural-risk findings.
- **Per-language resolution quality** in the summary (currently one aggregate
  ratio).
- **`--max-findings N`** for the CLI summary (`good first issue`).
- **Config JSON schema** so editors autocomplete `codegraph.config.json`
  (`good first issue`).

## Later

- **Dependency graph export** to Graphviz/DOT for embedding in docs.
- **Rule packs** so teams can share thresholds.
- **Incremental headless mode** that keeps a cache keyed by git HEAD.

## Good first issues

These are deliberately small and testable. If you pick one, say so on the issue
so two people do not duplicate the work.

1. **`--max-findings` flag** — cap the printed findings in
   `src/bin/cg_index.rs`, default 10, add a test in `tests/cli.rs` that asserts
   the line count. Files: `src/bin/cg_index.rs`, `tests/cli.rs`.
2. **Mermaid pipeline diagram** — document walk → facts → resolve → cache in
   `docs/ARCHITECTURE.md`. No code changes; verify it renders on GitHub.
3. **Python `src/` layout fixture** — add `tests/fixtures/py_src/` with a package
   under `src/` and assert the exact import edges in `tests/languages.rs`.
4. **Copy a single cycle** — the Cycles tab copies per group already; add a
   "copy all cycles" button using `report::cycle_text`. Files: `src/main.rs`.
5. **`index_quality` wording test** — assert the message never says "unused" or
   "dead" for any rule, to keep the phrasing honest. Files: `src/warnings.rs`.
6. **Config schema** — add `docs/codegraph.config.schema.json` and reference it
   from the README. No Rust changes.

## How items get here

Anything that needs a design decision starts as a discussion or an issue with
the problem, the proposed change, and the alternatives. The
[CONTRIBUTING guide](../CONTRIBUTING.md) explains the rest.
