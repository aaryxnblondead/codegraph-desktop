# Promotion playbook

Practical notes for getting the project in front of people who need it. The
short version: GitHub is a host, not a discovery engine. Distribution is a
separate job from building.

Everything here assumes the author discloses that they built the project,
respects each community's self-promotion rules, and does not ask for votes.

## Positioning

**One line (repo description):**
> Native Rust desktop app that indexes a codebase itself and reports coupling,
> cycles, impact, complexity, git hotspots and baseline regressions.

**Short pitch (2 sentences):**
> CodeGraph Desktop indexes a repo locally and turns the dependency graph into
> findings you can act on: the module that is unstable, the cycle to break, the
> file that is risky to touch. Findings name the symbol and line, and baselines
> show what got better or worse since the last run.

**Long pitch (paragraph):**
> Most dependency-graph tools stop at a picture. CodeGraph Desktop parses the
> project itself (TypeScript/JavaScript, Python, Rust), keeps the graph current
> while you edit, and reports module coupling and cohesion, file-level cycles
> with the cheapest edges to cut, change impact, complexity, git churn hotspots
> and hidden coupling. Every finding points at `file::symbol:line–end`, and a
> saved baseline turns "122 findings again" into "0 new, 12 fixed". There is a
> clipboard action for every pane and a headless `cg_index` that exits 2 on new
> findings, so the same checks can gate CI. No account, no network, and it never
> writes inside the project.

**Proof points** (use two or three, not all):

- 867 files: ~20 s cold, ~1.2 s warm, ~190 ms to load the saved graph.
- Save → updated graph with the edited file highlighted: ~30 ms.
- Findings read `app/admin/actions.ts::updateLeadDetails:231–366`, not "this
  file is complex".
- 141 tests, CI green on Windows/macOS/Linux, MIT.

## Assets in the repo

| Asset | Use |
|---|---|
| `docs/screenshots/demo.gif` | First thing in the README, post images, chat previews |
| `docs/screenshots/overview.png` | Social preview (crop to 1280×640), blog hero |
| `docs/screenshots/live-sync.png` | "It updates while you edit" posts |
| `docs/COMPARISON.md` | Answering "how is this different from X?" |
| `docs/FAQ.md` | Replies when someone asks whether the edges are trustworthy |
| `docs/ROADMAP.md` | Showing it is alive, and where to help |

## Channel playbook

| Channel | What to post | Caveats |
|---|---|---|
| **Hacker News — Show HN** | Title: `Show HN: CodeGraph Desktop – a local Rust app that finds what to fix first`. Body: problem, what it does, one screenshot, honest limits, link. | Post 08:00–10:00 US Eastern on a weekday. Reply to every comment for the first few hours. No vote asks. |
| **r/rust** | Title: `CodeGraph Desktop: a self-indexing Rust code explorer with coupling/cycle findings`. Body: what was hard (tree-sitter, resolution tiers, egui paint path), numbers, repo link. | Read the self-promo rules; participate before posting. Flair it correctly. |
| **r/programming** | The *problem*, not the repo: "I got tired of dependency graphs that stop at a picture". | Strict self-promo rules; often removes tool posts. Better as a discussion with the link in a comment. |
| **Dev.to / Medium / personal blog** | "I built a code explorer to answer one question: what should I fix first?" — the decisions and the measurements. Link the repo once, at the end. | Blog posts get more search traffic than the repo itself; repost the introduction, not the README. |
| **LinkedIn** | The project card + one screenshot + what you learned building it. | Tag the technologies, not people who did not ask. |
| **X / Bluesky** | 3-post thread: the problem, the demo GIF, the finding format + link. | `#rustlang #opensource #devtools`. |
| **Rust Weekly / This Week in Rust** | Submit the release or a blog post through their forms. | They need a link with a short description, not a pitch. |
| **awesome-rust / awesome-devtools** | PR adding it under the right section, with one accurate sentence. | Follow each list's contributing rules; never mass-add. |
| **Rust Discord / community forums** | Answer questions where the tool is genuinely relevant. | Only link when it solves the question being asked. |

## Ready-to-paste copy

### Show HN

**Title**
```
Show HN: CodeGraph Desktop – a local Rust app that finds what to fix first
```

**Body**
```
I kept bouncing between a dependency graph, a complexity report, and git log,
trying to answer one question: which part of this repo is expensive to change?

So I built a desktop app that indexes the repo itself (TypeScript/JavaScript,
Python, Rust via tree-sitter), keeps the graph current while you edit, and turns
it into findings:

- module coupling/cohesion (Ca, Ce, instability, abstractness, distance)
- file-level cycles, with the cheapest set of edges to cut (verified acyclic)
- change impact, complexity, git churn hotspots, hidden coupling, layer rules
- findings that name the symbol: `actions.ts::updateLeadDetails:231–366`
- baselines: "0 new, 12 fixed" instead of "122 findings again"

Numbers on an 867-file repo: ~20 s cold index, ~1.2 s warm, ~190 ms to load the
cached graph; a save re-indexes in ~30 ms.

Honest limits: resolution is conservative and says so (calls on external types
stay unresolved), only three languages, no LSP/network/cloud.

Rust + egui, MIT. Feedback welcome, especially on the resolution tiers and the
metric definitions: https://github.com/aaryxnblondead/codegraph-desktop
```

### r/rust

**Title**
```
CodeGraph Desktop: a self-indexing Rust code explorer with coupling and cycle findings
```

**Body**
```
I wanted a local tool that answers "what should I fix first?" without a SaaS
account, so I wrote one in Rust with egui.

It indexes the project itself (tree-sitter: TS/JS, Python, Rust), resolves
imports and symbols in tiers with explicit confidence, caches per-file facts by
content hash, and watches the folder so edits re-index in ~30 ms.

The analysis is the part I care about: coupling/cohesion per module, cycles with
the cheapest edges to cut, impact closures, complexity (JSX-aware, so render
functions do not dominate), git churn hotspots, hidden coupling, layer rules.
Findings point at `file::symbol:line–end`, and a baseline gives you new/fixed.

Things that were harder than expected: tiered name resolution without guessing,
the feedback-arc-set cut suggestions (verified before shown), and keeping the
egui draw path pure and bounded on a 10k-file repo.

141 tests, clippy -D warnings, CI on three platforms. MIT.
Repo: https://github.com/aaryxnblondead/codegraph-desktop
Happy to go deeper on any of the internals.
```

### X / Bluesky thread

```
1/ Dependency graphs usually stop at a picture. I wanted the next question:
   what should I fix first in this repo?

   So I built CodeGraph Desktop in Rust. It indexes the code itself and reports
   coupling, cycles with the edges to cut, impact, complexity and git hotspots.

2/ The detail that matters: findings name the symbol.

   `app/admin/actions.ts::updateLeadDetails:231–366` — 52 paths

   Not "this file has a complexity problem". [GIF]

3/ It also keeps score: save a baseline, then every run says
   "0 new · 12 fixed" instead of a count that only grows.

   Local, no account, MIT. 867 files: 20 s cold, 1.2 s warm.
   https://github.com/aaryxnblondead/codegraph-desktop
```

### LinkedIn

```
I built a desktop tool to answer one question: which part of a codebase is
expensive to change?

CodeGraph Desktop indexes a project locally (TypeScript/JavaScript, Python,
Rust) and reports module coupling and cohesion, dependency cycles with the
cheapest edges to cut, change impact, complexity, git churn hotspots and hidden
coupling. Findings point at the symbol and line range, and a saved baseline
shows new vs fixed findings between runs.

It is deliberately local: no account, no network, and it never writes inside the
project. Rust + egui, MIT, 141 tests, CI on Windows/macOS/Linux.

Repo: https://github.com/aaryxnblondead/codegraph-desktop
I would value feedback on the metric definitions and the resolution approach.
```

### Dev.to / blog outline

**Title:** `I built a code explorer to answer one question: what should I fix first?`

1. The problem: three tools, one question, no answer
2. Why a desktop app and not a SaaS dashboard
3. Indexing: tree-sitter, resolution tiers, and the confidence number
4. Why findings must point at symbols, not files
5. The two metrics I got wrong (god file fan-in, low cohesion volume)
6. Baselines: turning 990 findings into a 5-line diff
7. What is still hard (JSX complexity, framework routing, unresolved refs)
8. Try it / contribute

Tags: `rust`, `opensource`, `devtools`, `codequality`.

### Newsletter blurb

```
CodeGraph Desktop (Rust, MIT) indexes a repo locally and reports coupling,
cycles with suggested cuts, impact, complexity and git hotspots. Findings name
the symbol and line range, baselines show new vs fixed, and a CLI exits 2 on
regressions for CI. 867 files: 20 s cold, 1.2 s warm.
https://github.com/aaryxnblondead/codegraph-desktop
```

## Launch checklist

**Before**

- [ ] CI green on `main`; release tagged; README opens with the GIF.
- [ ] `gh repo view` shows the description and topics.
- [ ] Social preview uploaded (Settings → General → Social preview).
- [ ] Six `good first issue`s open with clear scope.
- [ ] Pin the repo on the profile.
- [ ] Record the baseline numbers you will quote (do not guess).

**During**

- [ ] Post to one channel at a time; give each a few hours.
- [ ] Reply to every comment for the first day.
- [ ] Turn repeated questions into FAQ entries the same day.
- [ ] File issues for real bugs found in the thread instead of arguing.

**After (48 h)**

- [ ] Write the follow-up post: what people asked, what changed.
- [ ] Merge the first community PR quickly and say thank you.
- [ ] Check traffic sources and double down on the one that worked:
  `gh api repos/aaryxnblondead/codegraph-desktop/traffic/views`
- [ ] Do not chase stars; watch issues, discussions and forks instead.

## Rules that keep it honest

- Disclose authorship in the first line where the platform allows it.
- Read the self-promotion rules before posting; when in doubt, ask a moderator.
- Never ask for upvotes, and never use alt accounts.
- Do not compare against a competitor in a way you cannot defend with a link.
- If someone reports a wrong result, reproduce it before replying.
