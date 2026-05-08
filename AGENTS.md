# AGENTS.md — contributor / AI agent workflow

This file is the workflow note for both human contributors and AI agents working on `cargo-rustics`. Read it once before adding a lens, fixing a bug, or submitting a PR.

## Working agreements

- **Conventional Commits.** Every commit follows https://www.conventionalcommits.org/en/v1.0.0/ . Use the right type — `feat`, `fix`, `docs`, `chore`, `test`, `refactor` — and scope when relevant: `feat(rustics): add cognitive-complexity lens`.
- **Self-application is a hard gate.** `cargo rustics analyze --fatal-warnings` runs against this repository in CI. If your PR adds code that violates a lens, you either refactor it, dismiss it with a documented reason in code, or relax the threshold in `rustics.toml` with the reason in the PR description. **Skipping the gate is not an option.**
- **Coverage floor 90 %, target 100 %.** CI runs `cargo llvm-cov --fail-under-lines 90 -p rustics`. Plan §12.5 sets 100 % as the ultimate target — every PR that adds code should add tests that keep us heading toward it. Dropping below 90 % fails the build.
- **Lens independence.** No lens depends on another. New lenses go in `crates/rustics/src/metrics/<id>.rs` and add to the `builtin_metrics()` enumeration; nothing else needs to change.
- **`syn` only at Layer 1.** Anything that needs type info is Layer 2 (M3) and lives behind a feature gate.
- **Conservative dependencies.** New dependencies need a one-line rationale in the PR description. `std` first, transitive second, new direct dep last (plan §1.8).
- **No copyleft.** MIT or Apache-2.0 only. `cargo-deny` enforces this in CI (plan §1.8.1, §0.5).

## Lens-addition recipe (plan §14)

1. **Visitor.** New file `crates/rustics/src/metrics/<id>.rs`; impl `MetricCalculator`. Re-export from `crates/rustics/src/metrics/mod.rs`. Register in `builtin_metrics()` in `crates/rustics/src/lib.rs`.
2. **Tests.** Unit tests live in the metric module — small, focused, one fixture per behaviour. Property-based tests welcome but not required.
3. **Rationale + refactor hints.** Write the `RATIONALE`, `REFACTOR_HINTS`, `REFERENCES` constants. Cite the original paper.
4. **Manual.** Add a section to `doc/manual.md` under "Lenses". Frame the lens, its threshold defaults, what "high" means, refactor hints, when to dismiss, references.
5. **Self-application.** Run `cargo rustics analyze --fatal-warnings` locally. If your own code violates the new lens, refactor or dismiss with reason.
6. **Caveats.** If the lens has a known blind spot, add it to `doc/manual.md`'s "Honesty about limits" section (mirrors plan §6.6).
7. **Commit.** `feat(rustics): add <name> lens`.

## Pull request template

Open the PR with:

```
## What

<one paragraph; what the PR adds or changes>

## Why

<one paragraph; the problem solved or the lens added>

## Self-application

- [ ] `cargo fmt --check` clean
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` clean
- [ ] `cargo test --all-features` clean
- [ ] `cargo rustics analyze --fatal-warnings` clean (or dismissals justified below)

## Dismissals

<list any dismissals added; each must include the reason>
```

## Working with AI agents on this repo

Recommended invocation:

```sh
cargo rustics manual | claude -p "I'm about to add a <name> lens. Sanity-check my plan against the manual."
```

The manual is the AI agent's first input. Don't paste the plan document into the model — the manual is the *embedded* plan, kept in sync with the binary by `include_str!`. If the manual is missing detail you need, it is a doc bug; fix the manual.

## Source layout

```
crates/rustics/         library; metric trait + lenses
crates/cargo-rustics/    CLI; reporters, analyzer, walker, config loading
doc/manual.md           embedded operator's manual
doc/ai-loop.md          end-to-end walkthrough for AI agents
tests/fixtures/         per-lens fixture inputs
tests/golden/           reporter golden tests (M1 onwards)
schemas/                JSON Schemas for the AI-report contract
rustics.toml             cargo-rustics's own configuration (self-application)
```

## Release flow (informal until 1.0)

`0.x` ships when M1 is complete and the self-application gate is green on `main`. Breaking changes to the AI-report contract bump the header (`# rustics ai-report v2`) and the minor version. Field *additions* do not break the contract.
