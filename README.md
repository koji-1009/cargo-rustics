# cargo-rustics

[![CI](https://github.com/koji-1009/cargo-rustics/actions/workflows/ci.yml/badge.svg)](https://github.com/koji-1009/cargo-rustics/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Rust code-quality metrics designed as the AI-loop counterpart of `cargo clippy`.

## What it does

cargo-rustics computes a battery of code-quality metrics — McCabe, Cognitive Complexity (Sonar), Chidamber & Kemerer, Hitz & Montazeri, Martin, Halstead, Nejmeh — on top of `ra_ap_syntax` (rust-analyzer-as-library), alongside a name-based public-API reachability heuristic that surfaces orphan `pub` items the compiler's `dead_code` lint cannot. Every report mode is shaped to be *consumed*: `--reporter ai` ships a token-efficient YAML-ish bundle, sorted by actionability, with each metric's rationale, refactor hints, and primary-source citation embedded inline.

The wager: the academic catalogue is reusable now in a way it wasn't before — not because the metrics changed, but because the consumer did. Humans cannot compute LCOM4 by eye; the number alone doesn't tell you what to change; even when it does, the refactor isn't free. An AI loop absorbs all three costs. The CLI computes in milliseconds, auto-explain ships the rationale alongside every violation, the agent does the edit, and `cargo rustics regression` confirms the metric actually settled.

Each metric is treated as a **lens**: one specific dimension of "hard to read", anchored to its original paper. Lenses are independent — a function can be clean by cyclomatic complexity and tangled by cognitive complexity. cargo-rustics does not gate; it surfaces what each lens reads, and leaves the accept / refactor / dismiss decision in the loop.

### Designed for the AI loop

- **Auto-explain by default** — rationale, refactor hints, and primary-source citation ride alongside every fired metric, so an agent reads the *why* without a second tool call.
- **Stable IDs across runs** — every violation carries a 16-hex-char id (`sha256("<file>|<scope>|<metric>")`), reappearing across runs so AI loops can detect "my fix didn't take". Surfaces as `partialFingerprints` in SARIF.
- **Docs in the binary** — `cargo rustics manual` and `cargo rustics ai-loop` print the operator's reference and the four-station walkthrough; `cargo install cargo-rustics` is enough, no separate doc download.
- **rustics measures, clippy lints** — orthogonal data shapes (numeric vs categorical), orthogonal stable-id semantics (function-scope vs file-line), orthogonal fix profiles (refactor vs `--fix`). Run them as separate CI steps; they compose.

## Install

```sh
cargo install cargo-rustics
```

## Quick start

```sh
# Token-efficient YAML-ish report optimised for LLM consumption.
cargo rustics analyze --reporter ai | claude -p "Refactor the threshold violations"

# After the agent applies a fix: confirm metrics actually improved.
cargo rustics regression --before HEAD~1 --after HEAD --reporter ai

# Read the operator's manual or the AI-loop walkthrough in the terminal.
cargo rustics manual
cargo rustics ai-loop
```

## Subcommands

| Command | Purpose |
| --- | --- |
| `analyze` | Run every enabled lens *and* the public-API reachability detector against the workspace. Combined report. |
| `regression` | Diff two snapshots; classify each delta as improved / regressed / unchanged / added / removed. Flags cosmetic refactors that move complexity around without removing it. |
| `unused [--apply]` | Public-API reachability only — the same data `analyze` includes, surfaced through a focused entry point that also offers `--apply` for in-place deletion. |
| `report <input.json>` | Re-emit a saved JSON snapshot in another reporter format. |
| `rules` | Catalogue every lens with rationale, refactor hints, and references. |
| `manual` | Print the operator's manual (mirrors [`doc/manual.md`](doc/manual.md)). |
| `ai-loop` | Print the AI-loop walkthrough (mirrors [`doc/ai-loop.md`](doc/ai-loop.md)). |
| `doctor` | Validate `rustics.toml`. |

Each subcommand only exposes the flags it actually consumes — `cargo rustics <command> --help` lists them. Full flag reference, dismissal mechanics, coverage / snapshot / regression details, and the refactor / dismiss decision tree all live in [`cargo rustics manual`](doc/manual.md) and [`cargo rustics ai-loop`](doc/ai-loop.md).

## Provided metrics

cargo-rustics ships a curated set anchored to published sources. For the audit trail — selection principles, deviations from the cited definitions, off-by-default rationale — see [`doc/calibration.md`](doc/calibration.md).

Each metric exposes `rationale`, `refactor_hints`, `references` (the primary source — McCabe 1976, Hitz & Montazeri 1995, Martin 1994, Drysdale 2024, …), and `polarity` (`LowerIsBetter` / `HigherIsBetter` / `Informational`). All four surface through `cargo rustics rules` and the AI / md / SARIF reporters so an agent can verify a metric against its original paper rather than paraphrasing from training data.

Lenses marked **off** ship disabled by default; opt in by adding `[rustics.metrics.<id>] warning = <n>` (and optionally `error = <n>`) to `rustics.toml`. A `—` in **Default warning** means the lens emits a measurement (so `regression` sees drift) but fires no warning until you set a threshold the same way.

### Function / method level

| Lens | Source | Default warning |
| --- | --- | --- |
| `cyclomatic-complexity` (sealed-aware) | McCabe 1976 | 10 |
| `cognitive-complexity` | Campbell / SonarSource 2018 — *industry source, not peer-reviewed* | 15 |
| `source-lines-of-code` | Boehm 1981 (industry convention) | 60 |
| `panic-density` (`unwrap_or`-aware) | Drysdale 2024, *Effective Rust* 2nd ed., Item 18 | 3 |
| `halstead-volume` *(off)* | Halstead 1977 | opt-in (recommended 1500; see [`doc/calibration.md`](doc/calibration.md)) |
| `npath-complexity` *(off)* | Nejmeh 1988 | opt-in (recommended 200) |

### Class / impl-block level

| Lens | Source | Default warning |
| --- | --- | --- |
| `wmc` (Weighted Methods per Class) | Chidamber & Kemerer 1994 | 50 |
| `lcom4` | Hitz & Montazeri 1995; Marinescu 2002 | 2 |
| `rfc` (Response For a Class) | Chidamber & Kemerer 1994 | 50 |

### Cross-file / module level (Martin 1994)

| Lens | Notes | Default warning |
| --- | --- | --- |
| `efferent-coupling` (per-file Ce) | Distinct external module roots a file imports. | 15 |
| `afferent-coupling` (cross-file Ca) | Workspace files that depend on this module. | 20 |
| `instability` (`I = Ce / (Ca + Ce)`) | Informational; surfaces drift in change-impact ranking. | — |

### Rust idioms (Drysdale 2024, *Effective Rust* 2nd ed.)

| Lens | Item | Default warning |
| --- | --- | --- |
| `unsafe-block-scope` | Item 16: Avoid writing unsafe code | 2 (lines) |
| `lifetime-arity` | Item 14: Understand lifetimes | 3 |
| `generic-arity` | Item 12: Understand trade-offs between generics and trait objects | 4 |
| `iterator-chain-length` | Item 9: Consider using iterator transforms instead of loops | 7 |

## Configuration

Minimal `rustics.toml` at the workspace root:

```toml
[rustics.metrics.cyclomatic-complexity]
warning = 10
error = 20

[rustics.metrics.cognitive-complexity]
warning = 15

[rustics.exclude]
patterns = ["tests/projects/**"]
```

Every key (per-metric thresholds, exclude patterns) is documented in [`schemas/rustics-config.schema.json`](schemas/rustics-config.schema.json) and explained in [`cargo rustics manual`](doc/manual.md). `cargo rustics doctor` validates the file in CI.

## Documentation

- [`cargo rustics manual`](doc/manual.md) — operator's reference: every flag, dismissal mechanics, refactor / dismiss decision tree, exit codes.
- [`cargo rustics ai-loop`](doc/ai-loop.md) — four-station walkthrough of one full refactor iteration with sample prompts.
- [`doc/calibration.md`](doc/calibration.md) — citation audit, selection principles, counting-rule deviations.
- [`schemas/`](schemas/) — JSON Schema files for the report and dismissal-sidecar formats (draft-2020-12).

## Output formats

`--reporter` accepts `console` (default), `json` (stable schema, see [`schemas/rustics-report.schema.json`](schemas/rustics-report.schema.json)), `md` (PR comments), `ai` (token-efficient YAML-ish bundle starting with `# rustics ai-report v1`), and `sarif` (SARIF 2.1.0 for GitHub Code Scanning).

## Embedding

The `rustics` library crate is intentionally tight — it exposes the function-level metric calculators so a custom CI bot or editor extension can compute one metric on a parsed `ra_ap_syntax::SourceFile` without spinning up the full CLI engine.

| What you get | Names |
| --- | --- |
| Function-level calculators | `CyclomaticComplexity`, `CognitiveComplexity`, `NpathComplexity`, `HalsteadVolume`, `SourceLinesOfCode`, `PanicDensity`, `UnsafeBlockScope`, `LifetimeArity`, `GenericArity`, `IteratorChainLength` |
| Class-level calculators | `Wmc`, `Lcom4`, `Rfc` |
| Module-level calculators | `EfferentCoupling` |
| Calculator interface | `MetricCalculator`, `MetricInput`, `MetricMeasurement`, `MetricMetadata`, `MetricCategory`, `MetricPolarity`, `MetricSeverity`, `Threshold` |
| Visitor helpers | `measure_functions`, `measure_impls`, `FunctionFrame`, `ImplFrame`, `FunctionKind`, `ScopeRef`, `ScopeKind` |
| Catalogue | `builtin_metrics()` returns every shipped lens; `violation_id(file, scope, metric)` for the stable hash |

Anything not in this table is CLI-only and unsupported as a Rust import; reach for `cargo rustics analyze --reporter json` instead.

## Auxiliary crates

- [`rustics-lsp`](crates/rustics-lsp) — LSP server publishing rustics diagnostics in your editor.

## Limitations

- **Built-in metric set is curated.** See [`doc/calibration.md`](doc/calibration.md) for selection principles. Lenses without a verifiable peer-reviewed or community-formal source (Effective Rust / Rust API Guidelines) are dropped rather than shipped on convention-only backing.
- **Per-file Martin granularity** is brittle for Rust. The release unit in Rust is the *crate*, not the file, and a `.rs` file can hold any number of `pub` items — Martin's 1994 framework assumes Java-style "1 public class per file" which doesn't hold. `efferent-coupling` / `afferent-coupling` / `instability` ship as change-impact rankings rather than Pain/Uselessness verdicts; `abstractness` and the derived Distance from Main Sequence are not provided for the same reason. See [`doc/calibration.md`](doc/calibration.md).
- **Layer-1 only.** Every lens reads the workspace through `ra_ap_syntax` (no type info, no borrow check). Heuristics that need types (e.g. resolving `self.method()` to a concrete impl) are intentionally out of scope; that work would be a Layer 2 (HIR via `ra_ap_hir`) and would live behind a feature gate.
- **Not a fit if** you need per-line metric thresholds in the IDE for the full metric suite, you don't engage with the dismiss channel at all (a pure-fail-fast linter is a better fit), or your codebase relies heavily on macro-expanded code that the un-expanded AST cannot see (run with `--expanded-macros` to mitigate).

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo llvm-cov --workspace --fail-under-lines 95
cargo rustics analyze --fatal-warnings
```

See [`AGENTS.md`](AGENTS.md) for the full contributor / AI-agent workflow notes.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
