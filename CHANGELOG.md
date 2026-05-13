# Changelog

## Unreleased

### Changed

- **`unused` detector now resolves names via HIR.** The Layer 1 AST walker is gone; `cargo rustics analyze` and `cargo rustics unused` always load the cargo workspace via `ra_ap_load_cargo` (rust-analyzer-as-library) and walk HIR for reference resolution. Fixes two false-positive classes the AST walker cannot avoid:
  - Homonym disambiguation across modules: two `pub fn helper` items in different modules, only one called — the AST walker credits both for the single `helper` token in the call site and flags neither; HIR resolves the path exactly.
  - Method calls inside macro bodies: `c.method()` invoked only via `eprintln!("{}", c.method())` — the AST walker doesn't enter macro bodies (and `cargo expand` leaves `format_args!` as a compiler-built-in macro un-expanded); HIR's macro server expands through.
  Architecture: `crates/rustics-ra/` ships the HIR walker; `cargo-rustics` depends on it as a regular (non-optional) dependency, no feature flag. Trade: install pulls ~170 transitive `ra_ap_*` crates (~50 s cold build), per-run analyze adds ~15-20 s of workspace load / macro server / cargo metadata over the AST-only baseline. The cost is comparable to `cargo clippy` on the same workspace.

### Fixed

- **`--expanded-macros` works on virtual-manifest workspaces.** The previous implementation ran `cargo expand --lib` once at the workspace root, which `cargo expand` rejects with "is a virtual manifest" on any multi-crate project (including this repository's self-application). The new implementation enumerates workspace members via `cargo metadata` and runs `cargo expand --lib -p <pkg>` for each lib target plus `cargo expand --bin <name> -p <pkg>` for each binary target, concatenating the captured stdouts. Partial-failure tolerant: a target whose `cargo expand` exits non-zero is skipped with stderr logged; the surviving targets' output still feeds the walker. With HIR now resolving the `unused` detector's macro-body cases, `--expanded-macros` is mostly useful for inspecting derive / proc-macro output through the AST metric lenses.

### Note on follow-ups

The strong-gain metric lenses (CC, Cognitive, NPath, LCOM4, RFC, the Martin coupling trio, sealed-aware match-arm-count) still walk `ra_ap_syntax`. Per the spike data, HIR resolves about a third of the lens catalogue more accurately (recursion across modules, aliased self, qualified-path method calls, re-export-aware coupling). Migration is in flight — see `tmp/hir-default-plan.md` for the per-lens status and order.

## 0.1.0

Initial release.

### Subcommands

- `cargo rustics analyze` — runs every enabled lens *and* the public-API reachability detector against the workspace and emits a combined report. Read-only; mutation lives in `unused --apply`. Supports `--reporter`, `--metric` / `--exclude-metric` filtering, `--fatal-warnings`, `--concurrency`, `--limit`, `--strict-dismiss`, `--coverage`, `--since`, `--expanded-macros`, `--snapshot-mode`, `--statistics`, `--no-auto-explain`, `--explain <metric-id>` (repeatable), `--filter <kind>` (narrows the unused-declaration list, same kind set as `unused --filter`), `-o` / `--output`.
- `cargo rustics regression` — diffs two snapshots and classifies each per-(scope, metric) delta as `improved` / `regressed` / `unchanged` / `added` / `removed`. Cosmetic-refactor heuristic (`tinyHelpersAdded ≥ 3 ∧ slocDelta > 4·helpers ∧ ccReduction < 2·helpers`) flags AI splits that move complexity around without removing it. `--before <baseline|cache|path>` resolves snapshot keywords to their canonical locations.
- `cargo rustics manual` — prints the embedded operator's manual (`doc/manual.md`).
- `cargo rustics ai-loop` — prints the embedded four-station walkthrough (`doc/ai-loop.md`).
- `cargo rustics rules` — lists every lens with its rationale, refactor hints, and references.
- `cargo rustics doctor` — validates `rustics.toml` (unknown ids, threshold ordering inconsistent with polarity, exclude-pattern shape).
- `cargo rustics report <input.json>` — re-emits a saved JSON snapshot in another reporter format.
- `cargo rustics unused [--apply]` — public-API reachability detector. The same detection runs inside `analyze`'s combined report; `unused` is the focused entry point that also offers in-place deletion via `--apply`. Reports unreferenced `pub` top-level items (`fn` / `struct` / `enum` / `trait` / `type` / `const` / `static` / `union`), every variant of a `pub enum`, and every `pub fn` / `pub const` inside an inherent `impl` block. References are credited via path last-segments, method calls, named field access, and `pub use` chain leaves. Roots: `fn main`, items with `#[test]` / `#[bench]` / `#[no_mangle]` / `#[export_name]` / `#[start]` / `#[proc_macro*]` / `#[ctor::ctor]` / `#[ctor::dtor]` / `#[xxx::main]`. `--apply` deletes top-level orphans in place (refuses on a dirty git tree without `--force`; skips `tests/` and `**/integration_test/**` without `--include-tests`; methods / variants / associated consts are reported but not yet auto-deletable). Honest limits: homonyms across modules under-report, proc-macro-generated identifiers under-report (run with `--expanded-macros` to suppress), recursive self-references count as references, and APIs consumed only by external crates surface as orphans by design.

### Reporters

- `console` — human-friendly summary line + per-violation lines.
- `json` — stable schema for `jq` pipelines and downstream tooling.
- `ai` — token-efficient YAML-ish bundle (`# rustics ai-report v1`), sorted by actionability, with rationale + refactor hints + references inline.
- `md` — Markdown for PR comments.
- `sarif` — SARIF 2.1.0 for GitHub Code Scanning / GitLab.

### Lens catalogue

14 lenses across:

- **Function-level (CS literature)**: `cyclomatic-complexity` (McCabe 1976, sealed-aware), `cognitive-complexity` (Campbell / SonarSource 2018, industry source not peer-reviewed), `source-lines-of-code` (Boehm 1981, industry convention), `panic-density` (`unwrap_or`-aware), `halstead-volume` (Halstead 1977, **off-by-default**), `npath-complexity` (Nejmeh 1988, **off-by-default**).
- **`impl` / `trait` shape (CS literature)**: `wmc` (CK 1994), `lcom4` (Hitz & Montazeri 1995), `rfc` (CK 1994).
- **Module coupling (Martin 1994)**: `efferent-coupling` (per-file), `afferent-coupling` (cross-file), `instability` (cross-file, informational).
- **Rust idioms (Drysdale 2024, *Effective Rust* 2nd ed.)**: `lifetime-arity` (Item 14), `generic-arity` (Item 12), `iterator-chain-length` (Item 9), `panic-density` (Item 18), `unsafe-block-scope` (Item 16).

Run `cargo rustics rules` for the live list with rationales and refactor hints.

Sealed-aware Cyclomatic Complexity: a `match` whose subject is an exhaustive enum (no `_` arm) does not count its arms — the compiler enforces exhaustiveness so the cognitive risk CC was designed to flag (a missed case) does not exist.

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

### Auxiliary crates

- **`rustics-lsp`** — LSP server publishing diagnostics in the editor.
- **`--expanded-macros`** — re-runs lenses on the cargo-expand output, capturing post-expansion shapes that proc-macros (`#[tokio::main]`, derive blanket traits) hide from the un-expanded source.

### Configuration

`rustics.toml` at the workspace root. Per-metric `warning` / `error` thresholds; `[exclude]` glob list; `[dismissal]` knobs (`requireReason`, `minReasonLength`); JSON Schema lives at `schemas/rustics-config.schema.json`.

### AI-report contract

`# rustics ai-report v1` header. Field renames or removals bump the header.
