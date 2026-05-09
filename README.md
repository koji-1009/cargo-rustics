# cargo-rustics

> Classical + Rust-specific code metrics for the AI coding loop.

`cargo-rustics` looks at Rust code through a stack of *lenses* — Cyclomatic Complexity, Cognitive Complexity, Halstead Volume, `clone-density`, `lifetime-arity`, `unsafe-block-scope`, and so on — and emits a report tuned for AI agents to consume. Each lens highlights one independent dimension of cognitive load or risk. Each violation carries a stable `id`, the rationale of the lens, and concrete refactor hints.

## Working rules

- **Every lens is citation-backed.** CS literature (CK, Martin, McCabe, Halstead, Hitz–Montazeri, Nejmeh, Sonar Cognitive Complexity) or community-formal sources (Effective Rust, Rust API Guidelines). "Something I noticed" is not a lens.
- **Lenses are independent.** A new lens lives in `crates/rustics/src/metrics/<id>.rs` and registers in `builtin_metrics()`; nothing else changes.
- **Multicollinearity is checked.** Pairs with `|r| ≥ 0.95` on self-application get dropped (Distance from Main Sequence was implemented and removed under this rule when it correlated `r=−0.994` with Instability).
- **Self-application is the shipping invariant.** `cargo rustics analyze --fatal-warnings` runs against this repository in CI; the tool can't ship if it fails its own lenses.
- **The AI loop is `manual → analyze → refactor → regression`.** All four are wired today.
- **The manual ships with the binary.** `cargo rustics manual` prints `doc/manual.md` via `include_str!`; install version and printed version cannot diverge.

## Quick start

```sh
cargo install cargo-rustics
cargo rustics manual                    # read the embedded manual
cargo rustics analyze --reporter ai     # see your code through every lens
```

## What ships today

Subcommands:

* `cargo rustics analyze` — runs every enabled lens against the workspace.
* `cargo rustics regression` — diffs two snapshots (improved / regressed / unchanged / added / removed) and flags cosmetic refactors.
* `cargo rustics manual` / `ai-loop` — print embedded operator docs.
* `cargo rustics rules` — list every lens with rationale + refactor hints.
* `cargo rustics explain <id>` — reverse-look-up a violation by its stable id.
* `cargo rustics doctor` — validate `rustics.toml`.
* `cargo rustics report <input.json>` — re-emit a saved snapshot in another reporter.
* `cargo rustics unused` — public-API reachability (Periphery-style).

Reporters: `console`, `json`, `ai`, `md`, `sarif`.

Lens catalogue: 30+ lenses across the function (`cyclomatic-complexity`, `cognitive-complexity`, `npath-complexity`, `halstead-volume`, `source-lines-of-code`, `maximum-nesting-level`, …), `impl` shape (`wmc`, `lcom4`, `rfc`, …), Martin coupling (`efferent-coupling`, `afferent-coupling`, `instability`, `abstractness`, `trait-impl-fanout`), and Rust-specific axes (`clone-density`, `unsafe-block-scope`, `panic-density`, `result-chain-depth`, `await-depth`, `borrow-profile`, `lifetime-arity`, `iterator-chain-length`, `boxed-allocation-density`, …). Run `cargo rustics rules` for the live list.

AI-loop integration: stable 16-hex violation `id`, auto-explain (rationale + refactor hints inline), `complexityJustified` for well-tested complex code, dismiss channel (sidecar TOML + doc comment, ≥ 20-char reasons, stale-entry detection), per-file snapshot (`cache` / `baseline`), `--since <ref>`, coverage gating, `--limit` for token-budget control.

Auxiliary crates: `rustics-macros` (`#[measured(cc < 10, …)]` compile-time gate), `rustics-build` (build.rs helper), `rustics-lsp` (LSP server publishing diagnostics in your editor), `--expanded-macros` (cargo-expand integration).

## How it composes with the rest of the toolchain

* **Clippy** — lints (rule violations). rustics — *lenses* (quantitative dimensions). Roles are orthogonal. Run them separately: `cargo clippy` for "is this wrong?" and `cargo rustics analyze` for "how complex is this?".
* **rust-analyzer** — type information. rustics's Layer 2 uses it for metrics that need real semantic data.
* **cargo-llvm-cov / cargo-tarpaulin** — coverage. rustics auto-detects `coverage/lcov.info` (or take `--coverage <path>`) and gates `complexityJustified` on branch / line coverage.

## Layout

```
crates/
  rustics/         library — MetricCalculator trait + lenses
  cargo-rustics/   CLI binary — analyze, regression, manual, …
  rustics-macros/  proc-macro: #[measured(cc < 10, …)]
  rustics-build/   build.rs helper that runs the analyzer at build time
  rustics-lsp/     LSP server publishing diagnostics
doc/
  manual.md        embedded manual (cargo rustics manual)
  ai-loop.md       end-to-end walkthrough for AI agents
schemas/
  *.schema.json    JSON schemas for the report contract
tests/
  fixtures/        per-lens fixture inputs (expected values in unit tests)
```

## Contributing

See [`AGENTS.md`](AGENTS.md) for the contributor / AI agent workflow note (including the lens-addition recipe) and [`CONTRIBUTING.md`](CONTRIBUTING.md) for the legal bits.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
