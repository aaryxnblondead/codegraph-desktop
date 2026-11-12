# Contributing to CodeGraph Desktop

Thanks for taking the time to contribute. This guide is meant to get you from a
fresh clone to a merged pull request without guessing.

- [Quick start](#quick-start)
- [What the project is](#what-the-project-is)
- [Where things live](#where-things-live)
- [Pick a first task](#pick-a-first-task)
- [Change recipes](#change-recipes)
- [Tests and quality gates](#tests-and-quality-gates)
- [Commits and pull requests](#commits-and-pull-requests)
- [Do's and don'ts](#dos-and-donts)
- [Getting help](#getting-help)

## Quick start

Prerequisites:

- **Rust** (stable, with `rustfmt` and `clippy`). Install with <https://rustup.rs>.
  No `rust-toolchain.toml` is committed on purpose: a host-specific channel
  overrides break other platforms. CI pins stable.
- **A C compiler** — the bundled SQLite and the tree-sitter grammars are C code.
  - Windows: "Desktop development with C++" from the Visual Studio Build Tools.
  - macOS: `xcode-select --install`.
  - Debian/Ubuntu: `sudo apt-get install build-essential`.
- **Git** (optional at build time, required for the git features).

Run it:

```bash
git clone https://github.com/aaryxnblondead/codegraph-desktop.git
cd codegraph-desktop
cargo run --release
```

The window opens with a demo graph. Press **Ctrl+O** (or *Open project*), pick a
folder, and it indexes that folder. No database or config file is required — a
`codegraph.db` is legacy support only.

Headless run (useful when you do not want a window):

```bash
cargo run --release --bin cg_index -- . --no-git
```

Point it at the repo you are in and you should see something like:

```text
41 files · 737 symbols · 2200 edges · 18% refs resolved · 402 ms
findings: 3 error · 29 warn · 9 info
by rule: low_cohesion 11, complex_function 9, main_sequence 7, god_file 5, …
```

## What the project is

A native Rust desktop app (egui) that indexes a project itself and reports design
problems: coupling and cohesion, cycles with the edges to cut, impact, complexity,
git churn, hidden coupling, layer rules, and baseline regressions.

Two properties shape most design decisions:

1. **The library is GUI-free except `src/ui.rs`.** Parsing, resolution, metrics,
   warnings, git and reporting are headless and tested without a window.
2. **The app never writes inside the project it indexes.** Caches, settings and
   baselines live under the OS app-data directory.

## Where things live

| Path | Responsibility |
|---|---|
| `src/indexer/` | walk → facts → resolve → cache pipeline |
| `src/indexer/lang/` | tree-sitter extractors (TypeScript, Python, Rust) |
| `src/metrics.rs` | coupling, cohesion, abstractness, complexity roll-ups |
| `src/warnings.rs` | rules, severities, locations, suppression |
| `src/structure.rs` | feedback arc sets (cycle cutting) |
| `src/impact.rs` | dependency closures and shortest paths |
| `src/git.rs` | churn, co-change, worktrees (read-only) |
| `src/baseline.rs` | snapshots and new/fixed diffs |
| `src/report.rs` | JSON/Markdown export and clipboard text |
| `src/ui.rs` | palette and custom-painted widgets (the only egui in the lib) |
| `src/main.rs` | the window, tabs, state, polling |
| `src/bin/cg_index.rs` | headless CLI |
| `docs/METRICS.md` | every formula, in plain language |
| `docs/FAQ.md` | common questions and fixes |

## Pick a first task

Issues labelled **`good first issue`** are scoped so you can finish one in a
sitting. Each one lists the files to touch and how to verify the result. Some
examples that are genuinely self-contained:

- Add a `--max-findings N` flag to `cg_index` (parse it, cap the printed list,
  add a CLI test).
- Add `cg_index --watch` that re-runs on file changes using `watch.rs`.
- Show the cycle's suggested cut edges in the *Impact* tab as clickable rows.
- Add a `docs/ARCHITECTURE.md` diagram (Mermaid) of the index pipeline.
- Add a fixture project under `tests/fixtures/` for Python `src/` layouts and
  assert the exact edges.
- Add a light theme and verify every text pair still meets 4.5:1 contrast.

If you are unsure whether something is in scope, open an issue first — a short
description of the problem and the intended change is enough.

## Change recipes

### Add a warning rule

1. Pick an id (`snake_case`) and decide severity.
2. Add the finding in `src/warnings.rs::run_warnings_reporting`. Fill
   `locations` with `Location::file(..)` or `Location::symbol(file, name, line, end)`.
3. Give it a one-sentence `explanation` and an actionable `suggestion`.
4. Add two tests: one graph that triggers it, one that does not.
5. Document it in `docs/METRICS.md` if it uses a new metric.

```rust
out.push(Warning {
    id: warning_id("my_rule", &[file.to_string()]),
    severity: Severity::Warn,
    rule: "my_rule".into(),
    message: format!("`{file}` does the thing"),
    subjects: vec![file.to_string()],
    count: 1,
    metrics: Default::default(),
    locations: vec![Location::file(file)],
    explanation: "Why this matters, in one sentence.".into(),
    suggestion: "What to try first.".into(),
});
```

### Add a language

1. Implement `LanguageExtractor` in `src/indexer/lang/`.
2. Register it in `src/indexer/lang/mod.rs` and gate it behind a feature.
3. Teach `src/indexer/resolve.rs` how its import specifiers map to files.
4. Add an extractor test and an end-to-end test under `tests/languages.rs`.
5. Bump `EXTRACTOR_VERSION` in `src/indexer/cache.rs` so old facts are dropped.

### Add a copy action

Formatting lives in `src/report.rs` (unit-tested), not in the UI. Add a
`fn foo_text(..) -> String`, test the exact string, then call
`copy_button(ui, &text, "copy foo")` from the panel.

## Tests and quality gates

Run these before pushing — CI runs the same set:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

Guidelines:

- A bug fix gets a test that fails before the fix.
- New rules get a firing case and a non-firing case.
- Resolvers and graph algorithms get a property-style check where practical
  (`tests/live.rs` and `src/structure.rs` show the pattern).
- Do not add `unwrap()` outside tests unless the value is provably infallible;
  add a comment when it is.
- Do not weaken an assertion to make a test pass. If the behaviour changed on
  purpose, update the test and say why in the PR.

## Commits and pull requests

Commit messages follow the conventional style used in the log:

```text
feat(warnings): point complex_function at the symbol
fix(indexer): reuse cached facts for unchanged files
docs: document the suppression config
test(git): cover detached worktrees
```

A good pull request:

- explains **what** changed and **why**, in two or three sentences;
- links the issue it closes (`Closes #12`);
- lists the commands you ran and their result;
- keeps unrelated refactors out — smaller diffs get reviewed faster;
- adds or updates tests and docs in the same PR.

We aim to respond to every PR and issue within a few days. If a PR goes quiet,
a polite ping is welcome.

## Do's and don'ts

**Do**

- Keep `src/main.rs` and `src/ui.rs` about presentation; analysis belongs in the
  headless modules.
- Bound everything that is drawn or listed (`MAX_LIST`, node caps) — the UI must
  stay responsive on a 10k-file repository.
- Prefer a small, testable function over a clever one.
- Keep user-facing text plain: short sentences, no marketing tone.

**Don't**

- Don't write inside the indexed project, and don't add network calls.
- Don't add `git` commands that mutate state (checkout, commit, push).
- Don't put egui types outside `src/ui.rs` and `src/main.rs`.
- Don't turn an index-quality number into a "dead code" claim — see
  `index_quality` in `src/warnings.rs` for the wording we use.

## Getting help

- Open a **question** issue or a discussion.
- Email the maintainer: **aaryansingh2810@gmail.com**.
- Security issues: see [SECURITY.md](SECURITY.md) — please do not file them
  publicly.

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).
