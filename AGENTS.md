# Agent Guidelines

Conventions for AI coding agents (Claude Code, Cursor, Codex, etc.) and human contributors working on this repository. Operational rules first; rationale second. If a rule lives elsewhere (CHANGELOG, README, `doc/manual.md`, `doc/calibration.md`), link rather than duplicate.

## Repository layout

- `crates/rustics/src/lib.rs` — public library API. Add `pub use` here when a new calculator or trait should be reachable from outside `crates/rustics/src/`.
- `crates/rustics/src/{input,measurement,metric,scope,visitor,identifier}.rs` — Layer-1 seams: `MetricInput`, `MetricMeasurement`, the `MetricCalculator` trait + metadata enums, `ScopeRef`, the `walk_*` / `measure_*` helpers, the `violation_id` hash. Every lens depends on these.
- `crates/rustics/src/metrics/<id>.rs` — per-scope metric calculators. Each implements `MetricCalculator` and provides `id`, `metadata`, `measure`, plus `RATIONALE` / `REFACTOR_HINTS` / `REFERENCES` constants. Register in `BUILTIN_METRIC_FACTORIES` in `lib.rs`.
- `crates/cargo-rustics/src/main.rs` + `cli.rs` — clap entrypoint. Defers to `commands/<name>.rs`.
- `crates/cargo-rustics/src/commands/{analyze,regression,manual,ai_loop,rules,doctor,report,unused}.rs` — one file per subcommand.
- `crates/cargo-rustics/src/cross_file/{mod,coupling}.rs` — workspace-level lenses (cross-file `afferent-coupling`, `instability`). Per-file lenses live in `crates/rustics/src/metrics/`; the cross-file pass runs after the per-file pass and merges into the same `Report`.
- `crates/cargo-rustics/src/{config,coverage,discover,dismissal,expanded,regression,report,runner,since,snapshot,statistics,workspace}.rs` — analysis pipeline pieces. `report.rs` owns the JSON-stable `Report` / `Violation` / `RustContext` shapes; `dismissal.rs` owns the sidecar TOML + doc-comment dismissal channels.
- `crates/cargo-rustics/src/reporters/{console,json,md,ai,sarif,mod}.rs` — output formatters. `mod.rs` dispatches to a reporter by `Reporter` enum.
- `crates/cargo-rustics/src/unused/{mod,apply}.rs` — name-based public-API reachability heuristic and the `--apply` deletion pass.
- `crates/rustics-lsp/src/main.rs` — LSP server publishing rustics diagnostics to editors.
- `doc/manual.md` / `doc/ai-loop.md` — operator's manual and AI-loop walkthrough; both are `include_str!`ed into the binary so `cargo rustics manual` / `cargo rustics ai-loop` always print the version that matches the installed CLI.
- `doc/calibration.md` — citation audit, selection principles, counting-rule deviations.
- `schemas/*.schema.json` — JSON schemas for the AI-report contract and the dismissal-sidecar shape (draft-2020-12).
- `tests/projects/` — hand-crafted fixture crates that fire specific lenses; excluded from self-application via `rustics.toml`'s `exclude` patterns.

## Workflow before every commit

Run, in this order, and address every finding:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo llvm-cov --workspace --fail-under-lines 95
cargo build --bin cargo-rustics && ./target/debug/cargo-rustics rustics analyze --fatal-warnings
```

- `cargo fmt` — never commit unformatted code; the CI `fmt --check` job rejects drift.
- `cargo clippy` — strict lints with `-D warnings`. Address info-level findings too.
- `cargo test --workspace` — every metric has a unit test; new metrics need one.
- `cargo llvm-cov` — workspace line-coverage gate at `95%`. Treat an uncovered line as evidence of dead code first; delete the unreachable branch before adding a contrived test.
- `cargo rustics analyze --fatal-warnings` — the dogfood gate (see below).

## Dogfood gate

CI runs `cargo rustics analyze --fatal-warnings` against this repository on every push and PR; the rustics codebase must clear its own metric battery to merge.

If a lens fires on idiomatic Rust code in this repo, the first move is **lens correction** (or its skip rule), not dismiss. A lens that over-fires on the canonical idiomatic-Rust codebase will over-fire elsewhere too — adjust the lens, not the call site.

Dismissal with reason is a fallback, not a default. The decision order is: **fix the code → fix the lens → drop the lens → dismiss**. The order matters: skipping straight to dismissal hides what is often a real bug in the lens or a real defect in the code.

## Code style

- snake_case filenames in `crates/*/src/`. Don't prefix files with underscore — Rust privacy is item-scoped, not file-scoped, so a leading-underscore filename gains no visibility benefit and just disrupts tooling.
- Match the surrounding code's voice. Don't introduce a new style or comment density alongside existing files.
- No defensive in-source comments when removing code. The "why" lives in the commit body and `git log`; leaving a tombstone comment in the source pollutes future reads.
- `forbid(unsafe_code)` is on for the library crate. The CLI may use `unsafe` only with a `// SAFETY:` comment explaining the invariant.

## Documentation conventions

- README, AGENTS, CHANGELOG, and everything under `doc/` are **English-only**. Conversation in issue threads or PR review can be any language; tracked artefacts stay English to keep the codebase consumable by international contributors and by AI agents trained on English corpora.
- One markdown bullet = one source line. Don't soft-wrap mid-sentence; let the renderer reflow.
- Don't reference `tmp/` paths from tracked files. The directory is gitignored and any reference would dead-end for fresh clones.
- README is "back of the box" — philosophy, what it does, the metric inventory at a glance. Detailed flag mechanics, dismissal protocol, and configuration reference live in `doc/manual.md` (mirrored as `cargo rustics manual`); the refactor walkthrough lives in `doc/ai-loop.md` (mirrored as `cargo rustics ai-loop`); citation audit lives in `doc/calibration.md`. When adding new operator detail, prefer `doc/manual.md` over README.

## Adding a new metric

1. Pick the right scope file under `crates/rustics/src/metrics/`.
2. Implement the calculator, anchoring its docstring to the original paper / spec. Don't paraphrase the formula; quote it.
3. Implement the metadata every metric must expose:
   - `id` — stable kebab-case identifier (used as JSON key and threshold key in `rustics.toml`).
   - `metadata().display_name` — human-readable label.
   - `metadata().category` — `MetricCategory` variant.
   - `metadata().polarity` — `LowerIsBetter` / `HigherIsBetter` / `Informational`.
   - `metadata().default_warning` / `default_error` — `Some(Threshold::new(_))` for thresholded lenses, `None` for informational.
   - `RATIONALE` const — one paragraph anchored in the original paper. Surfaces through `cargo rustics rules` and the AI reporter's auto-explain.
   - `REFACTOR_HINTS` const — list of single-sentence imperative refactor moves.
   - `REFERENCES` const — primary-source citations (paper / book / spec). Verify each citation against the original source; do not rely on secondary references. **A wrong citation is worse than no citation** — see [`doc/calibration.md`](doc/calibration.md) for the precedents we already corrected.
4. Register the calculator in `BUILTIN_METRIC_FACTORIES` in `crates/rustics/src/lib.rs`. Add the corresponding `pub use` re-export.
5. Add a unit test inside the metric module: a `measure(src)` helper that parses ra_ap_syntax + runs the lens, plus fixture-based assertions on the value and shape.
6. Update `README.md`'s "Provided metrics" table and `doc/manual.md`'s "Lenses" section.
7. If the metric deviates from its source's literal definition (e.g. sealed-aware CC, `unwrap_or`-aware panic-density), document the deviation in `doc/calibration.md`.
8. Update `CHANGELOG.md` under the next-release section.
9. Run `cargo rustics analyze --statistics` and verify no pair correlates `|r| ≥ 0.95` with a lens already in the catalogue. The multicollinearity rule is in `doc/calibration.md`'s "Selection principles".

## Adding a new reporter

1. Create `crates/cargo-rustics/src/reporters/<name>.rs` with a `write_with(report, opts, out)` entry point.
2. Add a `Reporter::<Variant>` to the enum in `crates/cargo-rustics/src/cli.rs` and route it in `crates/cargo-rustics/src/reporters/mod.rs::write_with`.
3. Allow `<name>` in the `--reporter` option's `value_enum` allowed set.
4. Add a golden test under `tests/golden/<name>_reporter/` exercising the reporter against a small fixture report.

## Configuration

The CLI reads from `rustics.toml` at the workspace root (override with `--config <path>`). When extending the schema, keep `crates/cargo-rustics/src/config.rs` in sync with `schemas/rustics-config.schema.json`. The `cargo rustics doctor` subcommand validates the schema; add a doctor check for any new key that has structural constraints.

## Commits

- Conventional Commits 1.0.0 — `<type>[scope]: <description>` plus body and footers when needed. Common types: `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `build`, `ci`, `perf`, `style`. Use `feat!:` (or a `BREAKING CHANGE:` footer) for breaking changes.
- One commit per logical unit. Don't bundle reformatting with feature work.
- Sign your commits if your local git is configured for signing.
- Keep commit messages and PR bodies free of pre-merge hygiene stamps (test counts, coverage percentages, "all tests pass", "dry-run clean"). Those belong in CI status, not in the historical artefact. Audit recent commits before composing — match the project's existing voice.
- AI-authored commits should carry a `Co-Authored-By: <model name> <noreply@anthropic.com>` footer (existing history uses `Claude Opus 4.7 (1M context)`; match the model the session is actually running).

## Pull request template

Open the PR with:

```
## What

<one paragraph; what the PR adds or changes>

## Why

<one paragraph; the problem solved or the lens added>

## Self-application

- [ ] `cargo fmt --all --check` clean
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean
- [ ] `cargo test --workspace --all-features` clean
- [ ] `cargo rustics analyze --fatal-warnings` clean (or dismissals justified below)

## Dismissals

<list any dismissals added; each must include the reason>
```

## Release flow

1. Bump `version` in the workspace root `Cargo.toml`. The version string flows into `env!("CARGO_PKG_VERSION")` so the binary's `--version` and the AI reporter's contract header stay in lockstep automatically.
2. Add a `## X.Y.Z` section to `CHANGELOG.md` covering every breaking change, citation correction, and feature.
3. Run `cargo publish --dry-run -p rustics` and `cargo publish --dry-run -p cargo-rustics` to confirm package contents.
4. The release commit is `chore(release): X.Y.Z` and is the final commit on a `release/vX.Y.Z` branch. Merge via PR.
5. Pre-1.0, breaking changes ship in minor versions (Cargo / SemVer 0.x convention). Mark them with `feat!:` and a `BREAKING CHANGE:` footer in the commit body. Field renames or removals on the AI-report contract bump the contract header (`# rustics ai-report v1` → `v2`).

## Scratch space

`./tmp/` is gitignored. Put plans, intermediate artefacts, and debug scripts there. Nothing under `tmp/` may be referenced from tracked files (README, source, comments) — those references would dead-end for fresh clones.

## Working with AI agents on this repo

Recommended invocation:

```sh
cargo rustics manual | claude -p "I'm about to add a <name> lens. Sanity-check my plan against the manual."
```

The manual is the AI agent's first input. It ships with the binary via `include_str!`, so `cargo rustics manual` always prints the version that matches the installed CLI. If the manual is missing detail you need, it is a doc bug; fix the manual.
