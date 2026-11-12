# Repository setup

The files in this repo are the source of truth, but GitHub's discovery surface
(description, topics, social preview, labels) lives in repository settings. This
page records the intended settings and the commands that apply them, so anyone
with admin rights can reproduce the setup.

Repository: `aaryxnblondead/codegraph-desktop` · public · MIT.

## Description

GitHub shows this under the repository name and in search results. Keep it one
sentence, outcome-first:

> Native Rust desktop app that indexes a codebase itself and reports coupling,
> cycles, impact, complexity, git hotspots and baseline regressions.

```bash
gh repo edit aaryxnblondead/codegraph-desktop \
  --description "Native Rust desktop app that indexes a codebase itself and reports coupling, cycles, impact, complexity, git hotspots and baseline regressions." \
  --homepage "" \
  --add-topic rust,code-analysis,static-analysis,dependency-graph,egui,tree-sitter,developer-tools,software-architecture,cli
```

Topics are the search surface: `rust`, `code-analysis`, `static-analysis`,
`dependency-graph`, `egui`, `tree-sitter`, `developer-tools`,
`software-architecture`, `cli`.

## Discussions

Questions and design ideas belong in Discussions; the issue templates link
there.

```bash
gh api -X PATCH repos/aaryxnblondead/codegraph-desktop \
  -f has_discussions=true
```

## Labels

The default GitHub labels are fine for triage. Add the ones this project uses:

```bash
for label in \
  "good first issue|7057ff|Scoped for a first contribution; files and verification listed" \
  "rule|1d76db|A warning rule or metric" \
  "indexer|5319e7|Parsing, extraction, resolution, cache" \
  "ui|d876e3|Interface, accessibility, rendering" \
  "git|c2e0c6|Churn, co-change, worktrees" \
  "docs|0e8a16|Documentation only"; do
  IFS='|' read -r name color desc <<< "$label"
  gh label create "$name" --color "$color" --description "$desc" --force \
    --repo aaryxnblondead/codegraph-desktop
done
```

## Social preview

Upload `docs/screenshots/overview.png` (Settings → General → Social preview).
GitHub recommends 1280×640; the screenshot is 1450×940, which crops acceptably,
or export a 1280×640 crop before uploading.

## Template and sponsorship

- **Template repository**: not enabled. This is an application, not a starting
  point. Enable it only if that changes.
- **Sponsors**: `.github/FUNDING.yml` points at the maintainer account; the
  Sponsor button appears once GitHub Sponsors is enabled for the account.

## Branch protection

Recommended for `main`:

- Require the CI checks (`fmt · clippy · test (ubuntu-latest)` at minimum) to
  pass before merge.
- Require a pull request, but allow the maintainer to merge their own after CI.
- Do not require a review for a solo maintainer; it blocks dependabot and
  small fixes.

```bash
gh api -X PUT repos/aaryxnblondead/codegraph-desktop/branches/main/protection \
  -F required_status_checks.strict=true \
  -f 'required_status_checks.contexts[]=fmt · clippy · test (ubuntu-latest)' \
  -F enforce_admins=false \
  -F required_pull_request_reviews=null \
  -F restrictions=null
```

## First release

After the first push:

```bash
git tag -a v0.1.0 -m "CodeGraph Desktop 0.1.0"
git push origin v0.1.0
gh release create v0.1.0 --title "v0.1.0" --notes-file CHANGELOG.md
```

Release binaries are not attached yet; `cargo build --release` produces
`codegraph_desktop` and `cg_index`. Attaching per-platform builds is a
[roadmap](ROADMAP.md) item.
