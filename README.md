# cargo-rustics

> Classical + Rust-specific code metrics for AI coding loops.

## What it does

cargo-rustics computes a battery of code-quality metrics — McCabe, Cognitive Complexity (Sonar), Chidamber & Kemerer, Hitz & Montazeri, Martin, Halstead, Nejmeh — on top of `syn`'s AST, alongside a name-based public-API reachability heuristic that surfaces orphan `pub` items the compiler's `dead_code` lint cannot (it only flags private items). Every report mode is shaped to be *consumed*: `--reporter ai` ships a token-efficient YAML-ish bundle, sorted by actionability, with each metric's rationale, refactor hints, and primary-source citation embedded inline. `console`, `json`, `md`, and `sarif` cover the human, code-review, and CI surfaces.

The wager: the academic catalogue is reusable now in a way it wasn't before — not because the metrics changed, but because the consumer did. Humans cannot compute LCOM4 by eye; the number alone doesn't tell you what to change; even when it does, the refactor isn't free. An AI loop absorbs all three costs. The CLI computes in milliseconds, auto-explain ships the rationale alongside every violation, the agent does the edit, and `cargo rustics regression` confirms the metric actually settled.

Each metric is treated as a **lens**: one specific dimension of "hard to read", anchored to its original paper. Lenses are independent — a function can be clean by cyclomatic complexity and tangled by cognitive complexity. cargo-rustics does not gate; it surfaces what each lens reads, and leaves the accept / refactor / dismiss decision in the loop.

- **Who it's for.** AI agents and the humans driving them.
- **What's different.** Metrics are signals, not gates: violations carry coverage data, a `complexityJustified` flag for well-tested complex code, and stable 16-hex-char ids you can dismiss with reasons.
- **Status.** Output formats carry contractual headers (`# rustics ai-report v1`); field renames or removals bump the header.
- **Docs in the binary.** `cargo rustics manual` prints the operator's manual ([`doc/manual.md`](doc/manual.md)); `cargo rustics ai-loop` prints the four-station walkthrough ([`doc/ai-loop.md`](doc/ai-loop.md)). Both ship with the executable, so `cargo install cargo-rustics` is enough — no separate doc download needed.
- **Calibration audit.** [`doc/calibration.md`](doc/calibration.md) is the audit trail for the lens battery — citations, counting-rule deviations, threshold calibrations, off-by-default rationale, intentionally-absent lenses, and outstanding audit gaps.

## Quick start

```sh
cargo install cargo-rustics
cargo rustics manual                    # read the embedded manual
cargo rustics ai-loop                   # the four-station walkthrough
cargo rustics analyze                   # default `console` reporter
cargo rustics analyze --reporter ai     # for piping into a coding agent
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
* `cargo rustics unused [--apply]` — name-based reachability heuristic over `syn`'s AST; surfaces unreferenced `pub` top-level items (`fn` / `struct` / `enum` / `trait` / `type` / `const` / `static` / `union`), every variant of a `pub enum`, and every `pub fn` / `pub const` inside an inherent `impl` block. `--apply` deletes top-level orphans in place (refuses on a dirty git tree without `--force`; skips `tests/` without `--include-tests`).

Reporters: `console`, `json`, `ai`, `md`, `sarif`.

Lenses span function-level complexity, `impl`/`trait` shape, module coupling (Martin), Rust idioms, macro shape, safety, and performance. Run `cargo rustics rules` for the full list with rationale + refactor hints.

AI-loop integration:

* Stable 16-hex violation `id` (`sha256("<file>|<scope>|<metric>")[..16]`).
* Auto-explain — rationale + refactor hints attached inline to every violation.
* `complexityJustified` flag — well-covered complex code is marked so the agent leaves it alone.
* Dismiss channel — sidecar `.rustics-dismissals.toml` or doc-comment (`/// rustics:dismiss <metric> reason="..."`), ≥ 20-char reasons, stale-entry detection. Sidecar wins on collision.
* Per-file snapshot (`cache` / `baseline`) for cosmetic-refactor detection.
* `--since <ref>` to scope output to changed files.
* Coverage gating (lcov auto-detect).
* `--limit <n>` for token-budget control.

Auxiliary crates:

* `rustics-lsp` — LSP server publishing diagnostics in your editor.
* `--expanded-macros` — re-runs lenses on the cargo-expand output.

## Layout

```
crates/
  rustics/         library — MetricCalculator trait + lenses
  cargo-rustics/   CLI binary — analyze, regression, manual, …
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

See [`AGENTS.md`](AGENTS.md). Release history is in [`CHANGELOG.md`](CHANGELOG.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
