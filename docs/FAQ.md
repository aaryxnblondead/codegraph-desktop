# FAQ and troubleshooting

## The app builds but the window does not open

- Check you have a **C compiler** installed. Bundled SQLite and the tree-sitter
  grammars need one. On Windows install the "Desktop development with C++"
  workload; the repo's `.cargo/config.toml` points rustc at the MSVC linker.
- Try `cargo run --release` rather than a debug run; the first build of
  egui/eframe takes a few minutes.

## "Only 18% of references resolved" — is the graph wrong?

Partly. Resolution is deliberately conservative:

- local scope → import bindings → globally unique name → uniquely named method.
- Calls on external types (`.iter()`, `.unwrap()`, `.clone()`, CSS class
  helpers) are not guessed, and neither are framework conventions the indexer
  does not know.
- The summary calls this out whenever a run resolves under 50% of references,
  and `index_quality` reports how many symbols have no resolved incoming edge.
  That number is an **index-quality** signal, not a dead-code list.

If you want stronger edges, open the project's `codegraph.config.json` and add
`entry_points` / module rules, or improve a resolver and send a PR.

## Why is this finding not pointing at a line?

Some rules are aggregate by nature: module instability, layer violations and the
volume note for `low_cohesion` apply to a group. Everything that has a symbol
(`complex_function`, `god_file`'s examples) carries
`file::symbol:line–end`.

## A rule fires on hundreds of files

Thresholds are configurable and the engine caps noisy rules:

- `low_cohesion` lists at most 10 files and adds one volume note.
- `complex_function` requires cyclomatic ≥ 20 (WARN) / ≥ 45 (ERROR) and never
  raises a render function above WARN.
- `god_file` needs a size floor (300 LOC or 40 symbols) before degree counts.

To accept a specific case, add a suppression:

```json
{ "suppress": [{ "rule": "complex_function", "path": "app/legacy/**", "reason": "vendor port" }] }
```

or inline:

```ts
// codegraph-ignore complex_function
```

## How do I compare two runs?

```bash
cg_index . --no-git --save-baseline baseline.json   # accept today's findings
cg_index . --no-git --baseline baseline.json        # later: new / fixed
```

The app does the same through *Set baseline* and the *Changed since baseline*
card.

## Does it write into my repository?

No. Caches and settings live under the OS app-data directory; baselines default
there too. The only file it reads from your project is an optional
`codegraph.config.json` you create yourself. It never commits, checks out or
pushes.

## Git features are missing

The app shells out to `git` and disables git features quietly when it is not a
repository or `git` is not on `PATH`. Install git and reopen the project. The
header shows `worktrees N · branches M` when it finds a repository.

## The graph is a hairball on a big repository

The overview renders at most 40 modules, file views at most 160 files and ego
views at most 80 symbols; the full graph is never drawn. Use the search box and
the drill-down instead of trying to read everything at once.

## Which languages are supported?

TypeScript/JavaScript (`.ts .tsx .js .jsx .mjs .cjs`), Python (`.py`) and Rust
(`.rs`). The extractor registry is designed to be extended — see
[CONTRIBUTING.md](../CONTRIBUTING.md#add-a-language).

## Can I use it in CI?

Yes:

```bash
cg_index . --json report.json --baseline baseline.json --fail-on-new error
```

Exit codes: `0` ok, `1` failure, `2` new findings at or above `--fail-on-new`.
