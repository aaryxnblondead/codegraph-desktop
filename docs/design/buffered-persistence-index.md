# Buffered-persistence indexing for `codegraph_desktop`

**Source:** Mingzhe Du and Michael L. Scott, *Buffered Persistence in B+ Trees*,
Proc. ACM Manag. Data 2(6) (SIGMOD), Article 226, December 2024.
<https://doi.org/10.1145/3698801>

**Question:** what of BD+Tree should this crate adopt, and how?

**Answer in one line:** adopt the *durability policy* — epoch-batched commits,
versioned slots, split-only growth, unsorted leaves with fingerprints — and not
the *NVM mechanism* (cache-line flushes, fences, HTM). On a desktop app the
persistent B+ tree is SQLite's job; the paper's value here is how the index is
*updated*, not how its pages are written to Optane.

---

## 1. What the paper actually does

A B+ tree on byte-addressable non-volatile memory has to survive a crash at any
instruction. Naive durable linearizability costs a cache-line write-back plus a
fence per update: the authors measure a 20–140 % throughput loss and up to 52×
write amplification on a cache-resident trace.

BD+Tree relaxes that. Execution is cut into *epochs* by a background clock
(50 ms on one socket, 500 ms across two). A crash in epoch `e` restores the
tree to the end of epoch `e−2`; an explicit `sync` advances the boundary by two
immediately. So persistence is *buffered*: writes are aggregated in cache and
flushed in bulk, and the last few milliseconds of index updates may be lost as
long as the surviving state is internally consistent.

Mechanisms that make that work:

| Mechanism | Role |
|---|---|
| Hybrid layout: inner nodes in DRAM, leaves in NVM | only leaves need persisting |
| 512 B unsorted leaves; first line is metadata, rest are KV slots | no key shifting on insert |
| Per-slot epoch number plus three bitmaps (`bitmap`, `bitmap−1`, `bitmap−2`) | visibility at `e`, `e−1`, `e−2`; recovery reads the right version |
| `tracked_lines` bitmap | each dirty cache line flushed once per epoch |
| Out-of-place update when the slot predates the current epoch | the old version stays behind for rollback; deletion is a cleared bit, not a write |
| `fingerprints[1 byte]` per slot | SIMD-filter candidate slots before comparing keys |
| Split, never merge; tombstones retained two epochs | simple, append-friendly growth |
| Writer announcement array with a double-check of the global epoch | every operation belongs to exactly one epoch |
| Background epoch thread + background reclaimer | flush and reclaim off the critical path |

Results: 1.1–2.4× throughput and up to 99 % fewer NVM writes when the working
set is cache-resident or temporally local. On a uniform workload whose working
set dwarfs the cache the win shrinks to 1–1.3× and costs ~12 % *more* writes,
because duplicate versions can no longer be combined in cache. Recovering a
10 M-pair tree takes ~200 s, because the inner tree is rebuilt by scanning the
persistent leaf layer.

## 2. What transfers to a desktop Rust app

There is no byte-addressable NVM here. Writes go through the OS page cache to
an SSD; durability comes from `fsync`, and SQLite already implements a
write-ahead log with atomic commits. Copying the flush/fence machinery would be
cargo culting — it would add unsafe mmap code and buy nothing.

What does transfer is the shape of the protocol:

| BD+Tree mechanism | Desktop reality | Adopt |
|---|---|---|
| `clwb`/`clflushopt` + `sfence` per epoch | no NVM; WAL does ordering | **No** — one transaction per epoch |
| Inner DRAM / leaf persistent split | graph in RAM, index on disk — already our split | **Yes**, unchanged |
| Epoch clock batches persistence | commit every N files or N ms | **Yes**, file-count driven |
| Writer announcement + epoch double-check | one writer thread; parse workers never write | **Simplify** to a batch id |
| Out-of-place versions | `created` / `deleted` epoch columns | **Yes** — also enables baselines |
| Three visibility bitmaps | two epoch columns + snapshot read | **Yes**, as columns |
| `tracked_lines` | SQLite WAL tracks pages; we track dirty *files* | **Partly** |
| Unsorted leaves | extractor emits file order; sorting is wasted work | **Yes** — in-memory buckets |
| 1-byte fingerprints | UI search currently lowercases every symbol | **Yes** — per-bucket filter |
| Split, never merge | file add/delete churn | **Yes** |
| Two-epoch rollback margin | atomic commit makes the window one open epoch | **No margin needed** (see §4.4) |
| HTM, seqlocks, per-leaf locks | single writer, WAL readers | **No** |
| Background reclaimer thread | prune dead versions inside the commit | **No thread needed** |

### 2.1 Workload fit — the part that decides the design

The paper's own numbers say where this pays: small working sets and temporal
locality. That is the *incremental* reindex path — a handful of files changed,
their leaf pages hot in cache. It is **not** the cold full index, which looks
like their U2 workload: uniform, larger than cache, where BD+Tree is 1–1.3× and
writes ~12 % more.

So the technique belongs on the delta path, not the primary store:

- **Full cold index** (876 files, 12.7 k symbols today): keep it as a batch
  build. SQLite inserts in one transaction are fast enough; a page-store B+tree
  would not beat it below ~10⁷ nodes.
- **Incremental reindex** (the common case, file save → graph refresh):
  epoch-batched, versioned, only the touched files rewritten.

That is also exactly where the crate has a gap: `index_project` builds a
`CodeGraph` in memory and throws the durability question away entirely.

## 3. Two implementation shapes

| Shape | What it is | Cost | Verdict |
|---|---|---|---|
| **A. Epoch index over SQLite** | versioned rows, one transaction per epoch, WAL snapshot reads | ~600 LOC, no new deps | **Recommended** |
| B. Literal port: 512 B page store + mmap + `fdatasync` | the paper's layout in Rust | ~2500 LOC, unsafe mmap, new dep, Windows file semantics, own recovery | Not justified below ~10⁷ nodes; keep as a benchmark-driven option |
| C. Status quo: in-memory only | nothing persists | 0 | Fails the crash-resume and baseline requirements already in the roadmap |

Shape A is what the rest of this note specifies.

## 4. Design: `IndexCache`

### 4.1 Location

`app_dirs().cache_dir()/projects/<project_root_id>/index.db`, following the
existing `ProjectConfig::config_path` pattern (`src/config.rs`). Never inside
the indexed project — that is a global constraint in the roadmap.

### 4.2 Schema

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;   -- batch durability; see §4.3

CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;
-- keys: schema_version, extractor_version, commit_epoch

-- Append-only symbol versions. A row is live when deleted IS NULL.
CREATE TABLE IF NOT EXISTS sym (
  id           TEXT    NOT NULL,   -- content-addressed, src/ids.rs
  epoch        INTEGER NOT NULL,   -- created
  deleted      INTEGER,            -- logical delete epoch, NULL = live
  path         TEXT    NOT NULL,
  kind         TEXT    NOT NULL,
  name         TEXT    NOT NULL,
  qname        TEXT    NOT NULL,
  line         INTEGER,
  end_line     INTEGER,
  signature    TEXT,
  doc          TEXT,
  exported     INTEGER NOT NULL DEFAULT 0,
  lang         TEXT,
  fp           INTEGER NOT NULL,   -- low 8 bits of blake3(name)
  PRIMARY KEY (id, epoch)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS sym_live_path ON sym(path, name) WHERE deleted IS NULL;
CREATE INDEX IF NOT EXISTS sym_live_name ON sym(name)        WHERE deleted IS NULL;

CREATE TABLE IF NOT EXISTS edge (
  id         INTEGER PRIMARY KEY,
  epoch      INTEGER NOT NULL,
  deleted    INTEGER,
  source     TEXT    NOT NULL,
  target     TEXT    NOT NULL,
  kind       TEXT    NOT NULL,
  line       INTEGER,
  confidence REAL    NOT NULL,
  provenance TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS edge_live_src ON edge(source) WHERE deleted IS NULL;
CREATE INDEX IF NOT EXISTS edge_live_dst ON edge(target) WHERE deleted IS NULL;
```

Two notes on the schema:

- *Why versions instead of `DELETE`/`UPDATE`.* A version row is never mutated;
  a change is `deleted = epoch` on the old row plus an insert. That is the
  paper's out-of-place rule, and it is what makes rollback and historical
  snapshots possible without an undo log. It also feeds `baseline.rs`
  (Phase 5): a baseline is a `commit_epoch`, not a copied graph.
- *Partial indexes are load-bearing.* The tombstone pass runs
  `UPDATE sym SET deleted = ? WHERE path = ? AND deleted IS NULL` once per
  staged file; without `sym_live_path` that is a full scan per file.

### 4.3 Commit protocol

```rust
pub struct EpochPolicy {
    /// Commit after this many staged files.
    pub max_files: usize,      // default 50 (matches the existing progress tick)
    /// Or after this much wall time, whichever comes first.
    pub max_millis: u64,       // default 50 (the paper's single-socket epoch)
}

pub struct IndexCache {
    conn:  rusqlite::Connection,
    epoch: u64,
    open:  Option<OpenEpoch>,
}

struct OpenEpoch {
    epoch:   u64,
    touched: HashSet<String>,   // dirty set: one write per file per epoch
}
```

One epoch:

1. `BEGIN IMMEDIATE`; `epoch += 1`.
2. For each staged file: mark its live rows deleted at `epoch`, insert the new
   versions, record the path in `touched`.
3. `COMMIT`; write `meta.commit_epoch = epoch` in the *same* transaction.

Because `commit_epoch` is written inside the transaction, a crash either loses
the whole epoch or keeps all of it — there is no state where rows exist but the
manifest disagrees.

`synchronous = NORMAL` in WAL mode is the durability knob, and it is the same
trade the paper makes. A *process* crash is always exact: the WAL tail is
checksummed and a partial frame is discarded. A *power* loss may drop the last
committed epochs, but the database still opens consistent. For a derived code
index that is the right trade — the worst case is that the user reindexes, and
`sync()` at the end of a run closes the window. `synchronous = FULL` buys
per-commit fsync at roughly one flush per epoch; measure before paying for it.

Use raw `BEGIN IMMEDIATE` / `COMMIT` / `ROLLBACK` via `execute_batch` rather
than holding a `rusqlite::Transaction`, which borrows the connection and would
make `IndexCache` self-referential.

```rust
impl IndexCache {
    pub fn open(root: &Path) -> Result<Self, CodegraphError>;
    pub fn begin_epoch(&mut self) -> Result<u64, CodegraphError>;
    pub fn stage_file(&mut self, rel: &str, facts: &FileFacts) -> Result<(), CodegraphError>;
    pub fn stage_delete(&mut self, rel: &str) -> Result<(), CodegraphError>;
    pub fn commit_epoch(&mut self) -> Result<u64, CodegraphError>;
    pub fn abort_epoch(&mut self) -> Result<(), CodegraphError>;
    pub fn durable_epoch(&self) -> Result<u64, CodegraphError>;
    pub fn sync(&mut self) -> Result<u64, CodegraphError>;
    pub fn load_symbols(&self) -> Result<Vec<Symbol>, CodegraphError>;
    pub fn load_edges(&self) -> Result<Vec<Relationship>, CodegraphError>;
}
```

`stage_file` is idempotent within an epoch: the dirty set checks the path first,
so a file touched twice is written once (the `tracked_lines` idea, at file
granularity).

### 4.4 Recovery

The paper needs `e−2` because a flush that started for epoch `e−2` may be
partially complete; the tree must tolerate any prefix of it. A SQLite commit is
atomic, so that hazard does not exist. **The window collapses to one open
epoch.** After a crash the database opens at the last commit the WAL could
vouch for; nothing is half-applied.

`open()` therefore does three cheap things, not a rebuild:

1. Read `meta.commit_epoch`; if absent, treat the cache as empty.
2. Verify `schema_version` and `extractor_version`; mismatch → drop the cache
   and rebuild (the roadmap's Task 2.3 rule).
3. Prune versions dead for more than one epoch:
   `DELETE FROM sym WHERE deleted IS NOT NULL AND deleted <= commit_epoch - 2`
   (same for `edge`).

That is O(dead rows), not the paper's 200 s per 10 M pairs. The `epoch` and
`deleted` columns still carry enough information to implement the real `e−2`
rule if the store later becomes a page file with asynchronous flush.

`sync()` = commit any open epoch, then `PRAGMA wal_checkpoint(TRUNCATE)`. Call
it when an index run finishes, so a clean exit leaves nothing buffered. This is
the paper's "explicit sync advances the boundary" (§3.1), used as an end-of-run
barrier.

### 4.5 Read path: buckets and fingerprints

This is the second transferable idea, and it is independent of durability.

`CodeGraph::find` today lowercases every symbol on every keystroke:

```rust
self.symbols.iter().filter(|s| s.name.to_lowercase().contains(&q) || ...)
```

That allocates a `String` per symbol per keystroke. Replace it with the paper's
leaf shape in memory — one bucket per file, unsorted, with a fingerprint byte:

```rust
pub struct Bucket {
    syms: Vec<Symbol>,        // append order = source order; never sorted
    fps:  Vec<u8>,            // fps[i] = fingerprint(syms[i].name)
}

pub struct FileIndex {
    buckets: HashMap<String, Bucket>,   // keyed by normalized relative path
}
```

- Insert is `push`; no `BTreeMap`, no re-sort. Key order inside a bucket does
  not matter because lookups compare fingerprints first, then the name.
- Search compares `fp` before touching the name. A one-byte fingerprint has a
  1/256 false-positive rate, so a bucket of 40 symbols filters to ~1–2
  candidates. False *negatives* are impossible, so correctness is unchanged.
- Deterministic output (export, baseline diff, tests) sorts a copy on demand,
  not on every insert.
- `by_id` should become `HashMap<SymbolId, u32>` indexing into `symbols`
  instead of `HashMap<SymbolId, Symbol>`. Today every symbol is stored twice
  (the `Vec` plus a cloned `Symbol` in the map); at 1 M symbols that is a
  pointless 2×.

This is exactly the `fingerprints` + unsorted-leaf combination from §3.2 of the
paper, minus SIMD. Its payoff here is search latency and memory, not write
throughput.

### 4.6 Concurrency

The indexer is already single-writer: one worker thread walks, parses and
resolves. So:

- Writer: the index thread owns `IndexCache`; parse workers send `FileFacts`
  over a channel, so tree/DB writes never happen on a rayon worker.
- Readers: the UI's `IndexStorage` opens its own read-only connection. WAL
  gives it a consistent snapshot at `commit_epoch` while the next epoch stages,
  which is the paper's "readers see a fixed epoch" property, for free.
- Cancellation: `abort_epoch` rolls the open epoch back. The durable prefix —
  everything already committed — stays usable, which is the roadmap's
  "cancellation mid-index: old cache and graph must remain usable" requirement.

## 5. Code changes

| File | Change |
|---|---|
| `src/indexer/cache.rs` | **new.** `IndexCache`, `EpochPolicy`, schema, recovery, loaders |
| `src/indexer/mod.rs` | stage into the cache per file; `commit_epoch` every `max_files`; `abort_epoch` on cancel; `sync()` at the end |
| `src/indexer/mod.rs` | `IndexProgress` gains `Committing { epoch, files }` (replaces the `Writing` placeholder) |
| `src/storage.rs` | `IndexStorage: Storage` over `IndexCache`, reading live rows at `commit_epoch` |
| `src/graph.rs` | `Bucket`/`FileIndex`, `find` via fingerprint prefilter, `by_id: HashMap<SymbolId, u32>` |
| `src/lib.rs` | re-export `IndexStorage` / `IndexCache` behind the existing `sqlite` feature |
| `src/main.rs` | open the cache on project load, show `commit_epoch` in the status line |
| `DECISIONS.md` | record: SQLite over custom page store; epoch size 50 files / 50 ms; window collapsed to one open epoch |

`src/ids.rs` needs no change. Content-addressed ids already give the stable key
the paper assumes, and are what make "delete old version, insert new version"
safe.

## 6. Tests

| Test | Asserts |
|---|---|
| `commit_is_atomic` | stage 3 files, commit, reopen → 3 files present |
| `crash_mid_epoch_keeps_prefix` | commit epoch 1, stage 2 more, drop without commit, reopen → only epoch 1 |
| `tombstone_hides_deleted` | delete a file, commit, reload → symbols gone, rows retained with `deleted` set |
| `incremental_equals_fresh` | index a fixture, change one file, incremental result == fresh full index (symbol and edge sets). Extends roadmap Task 2.7 |
| `epoch_size_invariance` | committing every 1 file vs every 50 yields identical final state |
| `fingerprint_no_false_negative` | property test over random names: every exact match survives the prefilter |
| `bucket_search_matches_linear` | property test: `FileIndex::find` == the old linear filter, for random buckets |
| `schema_version_forces_rebuild` | bump `schema_version` → cache dropped, no panic |

Crash simulation is "drop the connection without `COMMIT` and reopen", which is
the only failure mode a WAL database has.

## 7. Sequence

1. **Hot path first** — buckets, fingerprints, `u32` index. No persistence, no
   schema, immediately measurable in the UI.
2. **Cache core** — schema, `open`, `durable_epoch`, `sync`, loaders, tests 1–3
   and 7.
3. **Stage/commit/abort** — dirty set, tombstone pass, prune, test 5.
4. **Pipeline** — the epoch loop in `index_project`, `Committing` progress,
   cancel → `abort_epoch`.
5. **`IndexStorage`** — UI loads from the cache; incremental test 4.
6. **Incremental diff** — walk + `(mtime, size)` or blake3 per file; stage only
   changes, `stage_delete` for removals.
7. **Benchmark + docs** — fsync count and wall time at epoch sizes
   {1, 10, 50, 200 files}; record the numbers in `PROGRESS.md`.

## 8. Expected gains, stated honestly

- **Durability:** from zero (in-memory rebuild every launch) to an index that
  survives a crash and resumes. Commits cost `files / max_files` fsyncs, not
  one per file.
- **Incremental reindex:** one epoch writes only the changed files. This is the
  paper's favourable regime — expect the *shape* of their win, not the number;
  their 2.4× comes from eliding cache-line flushes that we never issue.
- **Search:** removes an allocation per symbol per keystroke. Negligible at
  12.7 k symbols, real at 10⁵–10⁶.
- **Not claimed:** any speedup on the cold full index. That workload is their
  U2, where BD+Tree is 1–1.3× and writes *more*.

## 9. Non-goals

- Byte-addressable NVM, `clflush`/`clwb`/`sfence`, DAX, wear-out.
- HTM, seqlocks, per-leaf locks — one writer, WAL readers.
- A custom page store. Revisit only if a benchmark shows SQLite is the
  bottleneck above ~10⁷ nodes; the epoch columns already support it.
- Node merges on delete. Split-only, as in the paper.
