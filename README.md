# cargo-rustics

> Classical + Rust-specific code metrics for the AI coding loop.

`cargo-rustics` looks at Rust code through a stack of *lenses* — Cyclomatic Complexity, Cognitive Complexity, Halstead Volume, `clone-density`, `lifetime-arity`, `unsafe-block-scope`, and so on — and emits a report tuned for AI agents to consume. Each lens highlights one independent dimension of cognitive load or risk. Each violation carries a stable `id`, the rationale of the lens, and concrete refactor hints.

## Seven things rustics asserts

1. **CS metrics from the 1970s–90s come back in the AI era.** Their cost was always *interpretation* and *action*; an AI loop pays that cost cheaply.
2. **Lenses multiply.** New lenses are added per release; each is independent and does not break existing ones.
3. **Rust × AI is the next coding market.** rustics aims to take the origin of that category before it crystallises.
4. **AI builds the quality device for AI's code.** The tool is implemented under the same loop it serves.
5. **A tool that does not pass its own output is not trustworthy.** rustics runs against itself in CI (`self-application gate`).
6. **The AI loop opens with `manual` and closes with `regression`.** They are core commands, not auxiliary.
7. **The tool carries its own manual.** `cargo rustics manual` prints the document `include_str!`'d into the binary at compile time.

## Quick start

```sh
cargo install cargo-rustics
cargo rustics manual                    # read the embedded manual
cargo rustics analyze --reporter ai     # see your code through every lens
```

## Status (M1)

What ships in 0.1.0:

* `cargo rustics analyze` — runs the M1 lens catalogue against a workspace.
* `cargo rustics manual` — embeds and prints `doc/manual.md`.
* `cargo rustics rules` — lists lens metadata.
* Reporters: `console`, `json`, `ai`.
* Stable violation id (`sha256("<file>|<scope>|<metric>")[..16]`).
* M1 lenses: `cyclomatic-complexity` (sealed-aware).
* `cargo rustics analyze --fatal-warnings` runs against this repository in CI (self-application gate).

What is on the roadmap:

* `cargo rustics regression` (M2) — verify an AI refactor is not cosmetic.
* `cargo rustics unused` (M3) — Periphery-style BFS for unused public API.
* Layer 2 metrics (M3) — `monomorphization-count`, `trait-resolution-depth`, `actual-borrow-cost`.
* Lens explosion: Cognitive Complexity, Halstead suite, `clone-density`, `lifetime-arity`, `unsafe-block-scope`, `panic-density`, `result-chain-depth`, `await-depth`, Martin's Ce/Ca/I/A/D, …

## How it composes with the rest of the toolchain

* **Clippy** — lints (rule violations). rustics — *lenses* (quantitative dimensions). Roles are orthogonal. Run them separately: `cargo clippy` for "is this wrong?" and `cargo rustics analyze` for "how complex is this?".
* **rust-analyzer** — type information. rustics's Layer 2 uses it for metrics that need real semantic data.
* **cargo-llvm-cov / cargo-tarpaulin** — coverage. rustics auto-detects `coverage/lcov.info` (or take `--coverage <path>`) and gates `complexityJustified` on branch / line coverage.

## Layout

```
crates/
  rustics/         library — MetricCalculator trait, M1 lenses
  cargo-rustics/   CLI binary — analyse, manual, rules, reporters
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
