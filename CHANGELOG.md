# Changelog

## 0.1.0

Initial release.

### Subcommands

- `cargo rustics analyze` — runs every enabled lens against the workspace and emits a report. Supports `--reporter`, `--metric` / `--exclude-metric` filtering, `--fatal-warnings`, `--concurrency`, `--limit`, `--strict-dismiss`, `--coverage`, `--since`, `--expanded-macros`, `--snapshot-mode`, `--statistics`, `--no-auto-explain`, `--explain <metric-id>` (repeatable), `-o` / `--output`.
- `cargo rustics regression` — diffs two snapshots and classifies each per-(scope, metric) delta as `improved` / `regressed` / `unchanged` / `added` / `removed`. Cosmetic-refactor heuristic (`tinyHelpersAdded ≥ 3 ∧ slocDelta > 4·helpers ∧ ccReduction < 2·helpers`) flags AI splits that move complexity around without removing it. `--before <baseline|cache|path>` resolves snapshot keywords to their canonical locations.
- `cargo rustics manual` — prints the embedded operator's manual (`doc/manual.md`).
- `cargo rustics ai-loop` — prints the embedded four-station walkthrough (`doc/ai-loop.md`).
- `cargo rustics rules` — lists every lens with its rationale, refactor hints, and references.
- `cargo rustics explain <id> [--snapshot <path>]` — reverse-look-up a violation id from a saved snapshot.
- `cargo rustics doctor` — validates `rustics.toml` (unknown ids, threshold ordering inconsistent with polarity, exclude-pattern shape).
- `cargo rustics report <input.json>` — re-emits a saved JSON snapshot in another reporter format.
- `cargo rustics unused [--apply]` — name-based reachability heuristic over `syn`'s AST. Reports unreferenced `pub` top-level items (`fn` / `struct` / `enum` / `trait` / `type` / `const` / `static` / `union`), every variant of a `pub enum`, and every `pub fn` / `pub const` inside an inherent `impl` block. References are credited via path last-segments, method calls, named field access, and `pub use` chain leaves. Roots: `fn main`, items with `#[test]` / `#[bench]` / `#[no_mangle]` / `#[export_name]` / `#[start]` / `#[proc_macro*]` / `#[ctor::ctor]` / `#[ctor::dtor]` / `#[xxx::main]`. `--apply` deletes top-level orphans in place (refuses on a dirty git tree without `--force`; skips `tests/` and `**/integration_test/**` without `--include-tests`; methods / variants / associated consts are reported but not yet auto-deletable). Honest limits: homonyms across modules under-report, proc-macro-generated identifiers under-report (run with `--expanded-macros` to suppress), recursive self-references count as references, and APIs consumed only by external crates surface as orphans by design.

### Reporters

- `console` — human-friendly summary line + per-violation lines.
- `json` — stable schema for `jq` pipelines and downstream tooling.
- `ai` — token-efficient YAML-ish bundle (`# rustics ai-report v1`), sorted by actionability, with rationale + refactor hints + references inline.
- `md` — Markdown for PR comments.
- `sarif` — SARIF 2.1.0 for GitHub Code Scanning / GitLab.

### Lens catalogue

30+ lenses across:

- **Function-level**: `cyclomatic-complexity` (sealed-aware), `cognitive-complexity` (Sonar 2018), `npath-complexity` (Nejmeh 1988), `halstead-volume` (Halstead 1977), `source-lines-of-code`, `maximum-nesting-level` (early-return-aware), `result-chain-depth`, `await-depth`, `panic-density` (`unwrap_or`-aware), `early-return-density`, `match-arm-count` (sealed-aware).
- **`impl` / `trait` shape**: `wmc` (CK 1994), `lcom4` (Hitz & Montazeri 1995), `rfc` (CK 1994), `trait-method-count`, `trait-default-impl-ratio` (informational), `impl-length` (informational).
- **Module coupling (Martin 1994)**: `efferent-coupling` (per-file), `afferent-coupling` (cross-file), `instability` (cross-file), `abstractness` (informational), `trait-impl-fanout` (cross-file).
- **Rust idioms**: `clone-density`, `lifetime-arity`, `generic-arity`, `closure-arity`, `iterator-chain-length`, `format-density`, `boxed-allocation-density`, `borrow-profile` (`owned` / `borrowed` / `mut`), `impl-trait-fanout` (informational), `dyn-density` (informational).
- **Macro**: `macro-rules-arm-count`, `proc-macro-presence`.
- **Safety**: `unsafe-block-scope`.

Run `cargo rustics rules` for the live list with rationales and refactor hints.

Sealed-aware Cyclomatic Complexity and Match-Arm-Count: a `match` whose subject is an exhaustive enum (no `_` arm) does not count its arms — the compiler enforces exhaustiveness so the cognitive risk CC was designed to flag (a missed case) does not exist.

### AI-loop integration

- **Stable violation `id`** = `sha256("<file>|<scope>|<metric>")[..16]`. Surfaces in JSON / AI / MD reporters and as `partialFingerprints.rustics/v1` in SARIF.
- **Auto-explain** (default on; `--no-auto-explain` to opt out) attaches each fired metric's rationale + refactor hints + references inline.
- **`complexityJustified` flag** marks CC / Cognitive violations whose scope has branch coverage ≥ 0.8 (or line ≥ 0.95 when BRDA records are absent). The agent can skip these.
- **Dismiss channel** — sidecar TOML (`.rustics-dismissals.toml`) or doc comment (`/// rustics:dismiss <metric> reason="..."`). `requireReason: true`, `minReasonLength: 20` are the defaults; reasons that fall short keep the violation live and stamp it with `dismissalRejected:`. Stale dismissals (no longer matching any live violation) surface as `staleDismissals:` in the report and as stderr warnings.
- **`--strict-dismiss`** ignores every dismissal — the raw triage list. CI / final-review use.
- **Snapshots (`--snapshot-mode cache | baseline`)** write a per-file `sha256` after each run and emit only records for files whose hash changed on the next invocation. Git-independent. `cache` lives at `target/.rustics-cache/snapshot.json`; `baseline` at `<workspace>/rustics-snapshot.json` for CI-shared baselines.
- **`--since <git-ref>`** filters output to declarations whose owning `.rs` file changed between `<ref>` and `HEAD`. Cross-file analysis stays accurate; only the *emitted* records are filtered.
- **Coverage gating** auto-detects `coverage/lcov.info` (or `--coverage <path>`) and attaches per-scope line + branch coverage to every violation.
- **`--limit <n>`** caps the AI / MD reporter's violation list (after the priority sort) for token-budget control.
- **`--statistics`** prints the lens-pair correlation matrix to stderr — used to guard against multicollinearity when adding a new lens.

### Multicollinearity rule

Lens pairs with `|r| ≥ 0.95` on self-application are dropped. `distance-main-sequence` was implemented and removed under this rule when it correlated `r = −0.994` with `instability`. `method-length` was dropped (`r = 0.984` vs `source-lines-of-code`). `impl-length` is informational-only (`r = 0.866` vs `wmc`).

### Auxiliary crates

- **`rustics-macros`** — `#[measured(cyclomatic_complexity < 10, lifetime_arity <= 2, ...)]` proc-macro that asserts every constraint at compile time and emits `compile_error!` on violation.
- **`rustics-build`** — build.rs helper that invokes the analyzer at build time so threshold violations fail the build.
- **`rustics-lsp`** — LSP server publishing diagnostics in the editor.
- **`--expanded-macros`** — re-runs lenses on the cargo-expand output, capturing post-expansion shapes that proc-macros (`#[tokio::main]`, derive blanket traits) hide from the un-expanded source.

### Configuration

`rustics.toml` at the workspace root. Per-metric `warning` / `error` thresholds; `[exclude]` glob list; `[dismissal]` knobs (`requireReason`, `minReasonLength`); JSON Schema lives at `schemas/rustics-config.schema.json`.

### AI-report contract

`# rustics ai-report v1` header. Field renames or removals bump the header.
