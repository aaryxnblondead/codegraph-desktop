# Metrics

Every formula here is a **heuristic** chosen for this app. None is mandated by a
standard. The conceptual basis for coupling/cohesion is the GeeksforGeeks
"Coupling vs Cohesion" article: coupling is the degree of dependency between
modules; cohesion is how closely the elements inside one module belong together.

Only edges with `weight > 0` and `confidence >= min_confidence` (default 0.5)
are counted. `contains` edges are structural and excluded.

## Coupling (between units)

A *unit* is a file or a module.

- `Ce(U)` — number of distinct other units `U` depends on.
- `Ce_edges(U)` — raw count of outgoing cross-unit edges (`ce_edges`).
- `Ca(U)` — number of distinct other units that depend on `U`.
- `Ca_edges(U)` — raw count of incoming cross-unit edges (`ca_edges`).
- `CBO(U)` — distinct units coupled to `U` in either direction.
- `instability I = Ce / (Ca + Ce)`; `None` when both are 0.
  0 = stable (many depend on it), 1 = unstable (depends on many).

Module-level `I` is the meaningful one. File-level `I` is reported but flagged
less often; leaf code is supposed to be unstable.

## Abstractness and the main sequence (module level)

- Type-like symbols: class, struct, enum, interface, trait.
- Abstract types: interface, trait (Python `Protocol`/`ABC` are not detected yet).
- `A = abstract / type_like`; `None` when there are no type-like symbols.
- `D = |A + I - 1|` when both exist. High `D` means a module is neither
  stable-and-abstract nor unstable-and-concrete. Treat it as a hint.

## Cohesion (within a unit)

- **File:** nodes are the file's symbols; edges are non-structural relationships
  between symbols in the same file (direction ignored). Report
  `components` (weakly connected components) and `largest_ratio`
  (largest component size / n). Many components means the file does several
  unrelated jobs.
- **Module:** nodes are the module's files; edges are cross-file edges whose
  ends are in the same module. Report `components` and `largest_ratio`.
- `internal_ratio = internal weight / (internal + outgoing weight)`.
- `internal_edge_ratio` keeps the older `internal / (internal + ce_edges)`.

## Complexity (per function, rolled up per file)

Cyclomatic = 1 + decision points:

- **TS/JS:** `if`, `for`, `for-in/of`, `while`, `do`, each `case` (not
  `default`), `catch`, ternary, `&&`, `||`, `??`.
- **Python:** `if`, `elif`, `for`, `while`, `except`, conditional expression,
  boolean operators, `match` cases, comprehension clauses.
- **Rust:** `if`, `while`, `for`, each `match` arm, `&&`, `||`. `?` is not
  counted.

Also recorded: max nesting depth, parameter count, function LOC. File rollups:
`complexity_sum`, `complexity_max`, `function_count`, `loc`.

## Hotspot score

`score = complexity × commits within the window` (files with no commits score
0; files with no functions use LOC instead of complexity). The CLI reports the
top 10 by this score; the Hotspots tab ranks by commit count. A strict
percentile version was specified but the product reads better with the plain
product, so the docs and code agree on the product.

## Logical coupling (co-change)

- Ignore commits touching more than 30 files.
- `support(a,b)` = commits touching both.
- `confidence(a->b) = support / commits(a)`.
- Report pairs with `support >= 5` and `confidence >= 0.6` in either direction.
- A pair with no direct static edge is a **hidden coupling candidate**
  (the UI checks direct edges, not transitive paths, for speed).

## Known blind spots

- Dynamic dispatch, reflection, and macros are not resolved.
- Low-confidence edges (`unique-name` 0.4, `method-name` 0.3) may be wrong.
  Both sit below the default `min_confidence` of 0.5, so they show in the graph
  but never drive coupling metrics.
- Generated code and vendored dependencies are included unless ignored.
- `contains` edges are excluded from coupling by design.
- In this repo's own Rust index only ~19% of refs resolve: most are method
  calls on standard-library or third-party receivers (`.iter()`, `.unwrap()`),
  which are deliberately not guessed. The index summary says so out loud when a
  run resolves under 50% of references.
