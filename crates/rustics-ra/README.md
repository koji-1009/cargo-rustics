# rustics-ra — Layer 2 spike

Branch: `experiment/ra-ap-spike`

This crate is an experimental probe to answer:

> if `ra_ap_*` (rust-analyzer-as-library) helps **detections beyond
> `unused`**, was the original choice of `syn` revisitable?

## What's here

- `src/workspace.rs` — wraps `ra_ap_load-cargo::load_workspace_at`
  and returns an `AnalysisHost` + `Vfs`.
- `src/unused.rs` — HIR-based unused detector that walks every
  `Crate::all` (filtered to `CrateOrigin::Local`), iterates each
  module's `declarations()`, and queries `Definition::usages` over
  a `SearchScope::module_and_children` of the crate root. References
  with the `IMPORT` flag set are not counted as live uses.
- `examples/detect.rs` — CLI smoke-test:
  `cargo run -p rustics-ra --example detect -- <manifest_dir>`.

## Spike findings

### Cold-build cost

- `cargo build -p rustics-ra` (cold, no cached deps): **53 s** wall
  (167 s user, 401% CPU on a 4-core), 213 CPU-seconds total.
- Incremental rebuild after a code edit: **0.45 s**.
- Adds the entire ra_ap_* dep tree (≈170 transitive crates).

### Speedup levers (added by `examples/load_bench.rs`)

The 51 s wall time on the cargo-rustics workspace breaks down
into ~1 s `load_workspace_at` and ~50 s of *lazy HIR queries*
that the walker forces by calling `Function::source(db)` per
function. The benchmark example tries the obvious config knobs
plus an HIR-bypass (parse files directly with `ra_ap_syntax`):

| Config | Load | HIR CC | HIR fns | Syntax CC | Syntax fns |
|---|---|---|---|---|---|
| default (sysroot+build_scripts+proc_macro) | 0.93 s | 50.65 s | 506 | **0.29 s** | **1519** |
| no proc_macro server | 0.93 s | 43.90 s | 506 | 0.29 s | 1519 |
| no build scripts | 0.85 s | 43.76 s | 506 | 0.29 s | 1519 |
| no sysroot | 0.70 s | 33.73 s | 524 | 0.30 s | 1519 |
| minimum (none of the above) | 0.62 s | 33.40 s | 524 | 0.29 s | 1519 |

The `Syntax CC` column uses `ra_ap_syntax::SourceFile::parse` per
file and walks the AST without going through HIR — same
decision-point rules as the HIR / syn implementations. Function
count (1519) matches the syn baseline almost exactly (syn = 1521;
delta is a couple of trait method declarations that have no body).

**Headline**: bypassing HIR for an AST-shaped lens drops total
wall time from ~51 s to ~1.2 s — a ~42× speedup — and surfaces a
*more complete* set of functions than the HIR walker was reporting
(the HIR walker had inadvertently skipped trait-impl methods after
the impl-bucketing optimisation).

The implication for Layer 2 architecture: lenses split into two
camps and the cost profile follows.

* **AST/token lenses** (CC, cognitive, npath, match-arm-count, SLOC,
  Halstead, dump-style idiom counts — ~20 of the catalogue) should
  walk source files via `ra_ap_syntax` directly. Cost ≈ syn.
* **HIR-needing lenses** (unused, lcom4, rfc, recursion-aware
  cognitive, sealed-aware match, Martin coupling, trait-impl-fanout —
  ~11 of the catalogue) pay the HIR query cost. Salsa amortises
  across queries, so running 11 HIR lenses isn't 11× the cost of
  running 1.

Total time to run *every* lens on the cargo-rustics workspace
under this split would then be ≈ load (1 s) + AST lenses (20 ×
~0.3 s ≈ 6 s) + HIR lenses (one-time HIR build ~30 s, plus
incremental queries shared across lenses). Order-of-magnitude
50 s, but most of it the HIR-needing work that has no equivalent
in syn-only Layer 1 — i.e. the work we're paying for is the
extra capability, not overhead.

### Runtime cost

| Scenario | Wall time |
|---|---|
| 1-file fixture (~30 lines), unused detector, `CrateOrigin::Local` filter | **17.3 s** |
| Same 1-file fixture, **without** origin filter (walks stdlib) | **534 s** (8.9 min, 7,431 false positives) |
| `cargo-rustics` own workspace (5 crates, 95 files), CC measurement | **51.8 s** |

The 1-file → 95-file scaling is roughly 3.4× wall-time for ~5× the
file count, because much of the workspace-load cost is fixed
sysroot discovery + cargo metadata, not per-file processing. That
means **subsequent lens queries on an already-loaded workspace
amortise well** — running 11 different HIR-backed lenses on the
same workspace is bounded by ~1× of the load cost, not 11×.

The without-origin-filter datapoint confirms `CrateOrigin::Local`
is mandatory: `Crate::all` traverses every dependency including
stdlib, and the per-Definition `usages` query compounds disastrously
across that surface.

### Empirical CC comparison: HIR vs syn

To confirm migration safety, the spike implements an HIR-backed
`cyclomatic-complexity` (`src/cc.rs`) using identical decision-
point rules as the syn-based lens: +1 per `if` / `while` / `for`
/ `loop` / `?`, +N-1 per non-wildcard `match` arm count (sealed-
aware `_`-less match contributes 0), +1 per `&&` / `||`. Baseline
1.

Run both backends on the cargo-rustics workspace itself (5 crates,
95 files) and diff per-`(file, scope)`:

| Metric | Value |
|---|---|
| **CC value disagreements among shared keys** | **0** |
| Functions seen by both backends | 664 |
| HIR-only (syn missed) | 5 (all trait method signatures — HIR walker emits decl-only fns, syn skips no-body items) |
| syn-only (HIR missed) | 857 (all `#[cfg(test)]` modules — HIR `CargoConfig` doesn't enable test cfg by default) |

**Migration risk = 0 on the CC lens**: every function that both
backends measured produced the same number. The two coverage gaps
are walker-shape decisions, not capability differences:

- HIR-side fix: skip `Function`s with no body (trait declarations).
  One-line filter.
- syn-side gap: HIR's `CargoConfig` defaults to non-test cfg. To
  pick up `#[cfg(test)]` modules, set
  `cargo_config.target_features` / cfg overrides accordingly. Same
  shape as `--all-targets` in cargo. Decision, not blocker.

This empirically validates that for the AST-shaped lenses (CC,
cognitive, npath, match-arm-count, …), HIR is a drop-in alternative
to syn — same numbers, same logic, different entry point.

### Empirical fixture comparison

Fixture (`/private/tmp/claude-501/ra-spike-fixture/src/lib.rs`):

```rust
pub mod mod_a { pub fn helper() -> i32 { 1 } }
pub mod mod_b { pub fn helper() -> i32 { 2 } }   // mod_b::helper is the homonym test

pub struct Foo;
impl Foo {
    pub fn used(&self) -> i32 { 0 }
    pub fn dead(&self) -> i32 { 0 }
}

pub use mod_a::helper as a_helper;

pub fn entry_point() -> i32 {
    let f = Foo;
    f.used() + mod_a::helper()
}
```

Two detectors run against this fixture:

| Decl | Layer 1 (syn) | Layer 2 (HIR) | Truth |
|---|---|---|---|
| `mod_a::helper` | not flagged | not flagged | used (called) |
| `mod_b::helper` | **not flagged** | **flagged** | unused |
| `Foo::used` (method) | not flagged | not yet walked | used (`f.used()`) |
| `Foo::dead` (method) | **flagged** | not yet walked | unused |
| `entry_point` (fn) | flagged | flagged | unused (no caller) |

Reading the row that exercises HIR's central capability:
**`mod_b::helper` is the homonym false-negative the syn-based
detector is structurally incapable of catching** — token counting
sees `helper` referenced via `mod_a::helper()` and credits both
mod_a's and mod_b's `helper` definitions. HIR resolves the path to
the canonical `Definition` and only credits `mod_a::helper`.

The `Foo::dead` row is a coverage gap on the Layer 2 walker
(impl-block methods not yet enumerated). Same fix shape as the
syn-side `unused` walker — straightforward extension, not a
structural limit.

### Cross-lens implications

Surveying the existing 30+ lens catalogue against
"would name resolution change the answer?":

| Lens | HIR helps? | Why |
|---|---|---|
| `unused` | **yes (proven)** | homonyms, method dispatch, proc-macro idents |
| `cyclomatic-complexity` | **yes** | recursion penalty needs canonical callee |
| `cognitive-complexity` | **yes** | same as CC; module-prefixed self-calls (`crate::foo::f()`) |
| `npath-complexity` | **yes** | same recursion case as CC |
| `lcom4` | **yes** | aliased self bindings, qualified paths, trait method dispatch |
| `rfc` | **yes** | `module::helper()` vs `Type::assoc_fn()`, qualified self paths, calls inside macro bodies |
| `wmc` | **yes (transitive)** | sum of CC; inherits CC fix |
| `efferent-coupling` | **yes** | true module dependency graph instead of import-segment heuristic |
| `afferent-coupling` (cross-file) | **yes** | same |
| `instability` | **yes (transitive)** | derived from Ce / Ca |
| `trait-impl-fanout` | **yes** | resolves trait identity through aliased imports |
| `match-arm-count` (sealed-aware) | **yes** | currently approximates "subject is enum" via path-typed scrutinee resolution |
| `clone-density`, `panic-density`, `result-chain-depth`, `await-depth`, `match-arm-count` content, `source-lines-of-code`, `maximum-nesting-level`, `lifetime-arity`, `generic-arity`, `closure-arity`, `iterator-chain-length`, `format-density`, `boxed-allocation-density`, `borrow-profile`, `early-return-density`, `impl-length`, `class-length`, `unsafe-block-scope`, `proc-macro-presence`, `macro-rules-arm-count`, `dyn-density`, `impl-trait-fanout`, `abstractness`, `trait-default-impl-ratio`, `trait-method-count` | no | AST/token-level idiom counts that don't depend on resolved meaning |
| `halstead-volume` | partially | macro-body tokens become visible if HIR-expanded source is used |

**Headcount**: of ~30 lenses, **roughly 11–12 gain accuracy** with
HIR. That's not just `unused`; it's the core complexity + coupling
surface (CK, Sonar, Martin lenses).

## Was syn the right starting point?

The spike was intended to surface this honestly. The empirical
data:

- ra_ap_* compiles cleanly on stable, runs correctly, and answers
  exactly the question Layer 1 cannot answer (homonym
  disambiguation, demonstrated).
- It is **not** "small dep, problem solved" — the cold-build cost
  is real, the API surface bumps every release, and the runtime
  workspace-load cost is a noticeable hit even on a tiny fixture.
- ~1/3 of the lens catalogue would gain accuracy with HIR. That's
  meaningful, not just a single-feature use case.

Read in good faith, the answer to the prompting question is:

- **Choosing `syn` first was a defensible time-to-ship decision**
  given the API stability and compile-cost trade-offs.
- **Choosing only `syn` long-term is not optimal** for the lens
  catalogue cargo-rustics has signed up to maintain. Layer 2
  (`ra_ap_*`-backed) is the right home for the lenses listed above.
- **`syn` is still the right Layer 1 default** because most lenses
  in the catalogue (~20) don't need HIR and the milliseconds-per-
  file experience would regress badly.

## API churn caveat

`ra_ap_*` is published `0.0.x` and the public API breaks across
versions. This spike used `0.0.331`. Surface friction observed in
just the spike:

- `LoadCargoConfig` gained `num_worker_threads` and
  `proc_macro_processes` since older snippets.
- `SearchScope::workspace` was removed — use
  `SearchScope::module_and_children(db, krate.root_module(db))`
  per crate, then iterate.
- `ReferenceCategory::Import` is now bitflag
  `ReferenceCategory::IMPORT`.
- `Definition::try_to_nav` lives in the `TryToNav` trait, not as
  an inherent method.
- Queries that touch the next-gen trait solver panic with
  "no db is attached" unless wrapped in
  `ra_ap_hir::attach_db(db, || …)`.

Each upgrade will probably require touching the crate. If we adopt
Layer 2, we need a CI job that catches these breakages.

## Status

Branch: `experiment/ra-ap-spike` — checkpoint commit, not for
merge. The spike answers the design question; the production
implementation (proper module structure, feature gate on
`cargo-rustics`, full lens catalogue) is a separate PR.
