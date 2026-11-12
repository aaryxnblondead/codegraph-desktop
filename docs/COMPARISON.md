# How this compares

Tools below are good at what they do. The point of this table is to show where
CodeGraph Desktop fits, not to claim it replaces them. If your need is one row,
use that tool.

| | CodeGraph Desktop | [madge](https://github.com/pahen/madge) / [dependency-cruiser](https://github.com/sverweij/dependency-cruiser) | [CodeScene](https://codescene.com) | [Understand](https://scitools.com) | [Sourcegraph](https://sourcegraph.com) |
|---|---|---|---|---|---|
| **Where it runs** | Local desktop app | Local CLI | SaaS / server | Local desktop | Server / cloud |
| **Account or network** | None | None | Account | License | Account |
| **Languages** | TS/JS, Python, Rust | JS/TS | Many | Many | Many |
| **Graph you can click** | Yes, bounded module/file/ego views | No (reports, DOT/SVG) | Web dashboards | Yes | Code search + graph |
| **Coupling / cohesion metrics** | Ca, Ce, instability, abstractness, distance, cohesion groups | Partly (dependency-cruiser rules) | Yes, behavioural | Yes | Limited |
| **Cycles with the edges to cut** | Yes, verified feedback arc set | Lists cycles | Lists | Lists | Lists |
| **Findings point at symbols** | `file::fn:line–end` | Rules point at modules | Yes (hotspots) | Partial | No |
| **Baselines / new vs fixed** | Yes, `0 new · 12 fixed` | No | Trend charts | Snapshots | No |
| **Git churn + hidden coupling** | Yes (read-only CLI) | No | Yes (its core) | No | Limited |
| **CI gate** | `cg_index --fail-on-new` | Yes | Yes | No | No |
| **Cost** | Free, MIT | Free | Commercial | Commercial | Free tier / commercial |

## What that means in practice

- **You want a dependency picture of a JS repo in CI.** `dependency-cruiser` is
  more mature and has richer rule syntax. Use it.
- **You want behavioural code health across a large org, with history and
  dashboards.** CodeScene is built for that and has years of research behind it.
- **You want a local, account-free tool that answers "what should I fix first in
  this repo, and did my change make it better?"** — that is the gap this project
  targets: a clickable graph, findings that name the symbol and line, and a
  baseline diff instead of a count that only grows.
- **You want multi-language in one view.** The three supported languages are
  parsed by the same pipeline, so a mixed repo produces one graph. Other tools
  often specialise per language.

## Honest limits

- Resolution is conservative: calls on external types and unknown framework
  conventions stay unresolved, and the summary says so. `dependency-cruiser`
  and `Understand` resolve more in their supported languages.
- There are three languages. The extractor registry is designed to be extended,
  but it is not a plug-in marketplace.
- No LSP, no network, no cloud sync. That is deliberate.
- The DSM is a builder only; the matrix view is still on the roadmap.

If a claim here is wrong or unfair, open an issue — corrections are welcome and
will be made quickly.
