# cargo-rustics — operator's manual

> **Audience:** the AI agent driving a Rust refactor loop.  
> **How to read this:** top to bottom, once. Re-read the lens you triggered when you reach a violation.  
> **How to obtain this:** `cargo rustics manual` — this file is embedded in the binary at compile time. Network is not required.

---

## TL;DR

```sh
cargo rustics analyze --reporter ai          # see code through every lens
cargo rustics manual                          # read this document
cargo rustics regression --before HEAD~1 --after HEAD   # M2 — verify a refactor
```

The AI loop is **manual → analyze → refactor → regression**. `manual` is the entry; `regression` (shipping in M2) is the exit. `analyze` is the body. Today (M1), only `manual` and `analyze` are wired.

---

## What rustics is, in one paragraph

Rustics looks at your Rust code through a stack of *lenses* — Cyclomatic Complexity, Cognitive Complexity, Halstead Volume, `clone-density`, `lifetime-arity`, `unsafe-block-scope`, and so on. Each lens highlights one independent dimension of cognitive load or risk. Each lens is implemented in `crates/rustics/src/metrics/<id>.rs` and walks the same `syn` AST. The CLI runs every enabled lens, attaches a stable id to every violation, and emits a report tuned for AI consumption (`--reporter ai`). The output is signal, not a gate: *every* violation can be dismissed with a stated reason — but it must be *stated*.

## Why a lens, not a score

A score collapses dimensions. A lens names them. When CC is high, the question is "is the function branchy because of business rules, or because no one extracted the early returns?". When `clone-density` is high, the question is "is the function cloning to escape the borrow checker, or because it owns short-lived strings?". Different lenses, different refactors. A score blurs that.

## The decision triangle

Every violation lands on one of three outcomes:

```
                  accept ────── (no change; signal noted but acceptable)
                 ╱
violation ──────╳────── refactor ── (apply a hint, re-run analyze)
                 ╲
                  dismiss ─── (annotate with reason; sidecar TOML or doc comment)
```

Pick deliberately. Don't dismiss to silence. Don't refactor to game.

---

## Lenses (M1 catalogue)

> The catalogue grows every release. Run `cargo rustics rules` for the live list.

### `cyclomatic-complexity` (sealed-aware)

**What it sees.** Linearly independent paths through a function. Branches, loops, `?`, `&&`/`||` each add `+1`. `match` on a non-wildcard arm set contributes `0` because the compiler is checking exhaustiveness for you (sealed-aware adjustment, plan §2.5). `match` *with* a `_` arm contributes `arms - 1`.

**Default thresholds.** warning `10`, error `20`.

**What "high" means.** A function with CC > 10 has more decision points than most readers can hold in working memory in one sitting. Test combinatorics also explode — fully covering N independent branches needs ≥ N test cases.

**Refactor hints.**
1. Extract one branch arm into a helper. The helper carries its own preconditions, the caller stays linear.
2. Replace nested `if`/`else` chains with a `match` on a small enum. The sealed-aware rule then absorbs the branches.
3. Lift early-return guards to the top with `let … else { return … }` so the happy path stays on the spine.
4. If the function is a state machine, name the states: each state is its own small function with a low CC.

**When to dismiss.** Recursive descent parsers, hand-rolled lexers, finite-state machines whose states are intrinsic to the domain. Reason field: write *why splitting hides intent*.

**References.** McCabe 1976; plan §2.5.

### `source-lines-of-code` (SLOC)

**What it sees.** Non-blank, non-comment-only lines inside a function body.

**Default thresholds.** warning `60`, error `120`.

**What "high" means.** Long bodies hide what they do. SLOC is the conservative size measure: low SLOC + high CC means dense; high SLOC + low CC means sprawl.

**Refactor hints.**
1. Extract a contiguous block into a named helper. The helper's name is documentation.
2. Lift `let` chains and conversions to the top so the body's shape is visible.
3. Replace long `if`/`else` chains with a `match` (the sealed-aware CC adjustment makes this free at the CC lens).

**References.** plan §2.3.

### `method-length`

**What it sees.** Total physical line count from `fn` to closing `}` (signature + body).

**Default thresholds.** warning `80`, error `160`.

**What "high" means.** Coarser than SLOC, but it captures what a reader actually scrolls past. The gap between method-length and SLOC is the *signature weight*: if it's wide, the signature is doing a lot of work (`where` clauses, multi-line `impl Trait`).

**Refactor hints.**
1. Wide gap with SLOC → consider a type alias or builder so the signature reads in one line.
2. Small gap with SLOC but high method-length → the body is long; extract helpers as for SLOC.

**References.** plan §2.3.

### `number-of-parameters`

**What it sees.** Positional parameter count of the signature, excluding `self`. Trait-required methods are measured (signature-only).

**Default thresholds.** warning `5`, error `8`.

**What "high" means.** Each positional parameter is a fact the caller has to remember and a slot they can mis-order. Past 4–5, callers start passing wrong cells. Rust has no call-site keyword arguments, so positional-arity *is* the contract the user reads.

**Refactor hints.**
1. Group co-occurring parameters into a struct — the struct's fields document themselves.
2. If a parameter is always the same at most call sites, hoist it to the receiver type or a builder.
3. Replace a positional `bool` with an enum so the call site reads `Mode::Strict`, not `true`.

**References.** plan §6.1.

### `maximum-nesting-level` (early-return-aware)

**What it sees.** Deepest nesting reached inside a function body. Each entry into an `if` / `while` / `for` / `loop` / `match` body adds `+1`. Two Rust-aware refinements (plan §2.5):

* `else if` chains read flat (siblings, not nested).
* When an `if let X { … } else { … }` else branch diverges (`return`, `panic!`, `bail!`, …) the whole construct is treated as transparent — the same shape `let-else` would have if Rust allowed pattern binding there.

**Default thresholds.** warning `4`, error `6`.

**What "high" means.** Past 4 levels, unwinding the meaning back to the function's intent costs real attention. Each level forces the reader to hold one more `if`/`for`/`match` precondition.

**Refactor hints.**
1. Lift `if let X else { return }` style guards to the top — the body that follows stays linear and the metric drops.
2. Extract the inner-most loop or block into a helper. The deepest level becomes the helper's depth-1 body.
3. Replace nested `match` with `if let` early-return guards followed by a flat `match` at the function's top.
4. Use `?` instead of `match Result + return Err(...)`.

**References.** plan §2.5.

### `lifetime-arity`

**What it sees.** Number of explicit lifetime parameters on a function signature. Implicit elision is not counted — that is the point of elision.

**Default thresholds.** warning `3`, error `5`.

**What "high" means.** Each lifetime is one referential constraint the reader has to track. Past three, the signature is a small constraint puzzle that has to be solved before the call.

**Refactor hints.**
1. Push the lifetimes into a struct (`struct Borrow<'a> { ... }`); the function takes `Borrow<'_>` instead.
2. Take ownership where possible — `String` instead of `&'a str`.
3. Many signatures with explicit lifetimes are eligible for elision; try removing them and let rustc tell you.

**References.** plan §2.4.

### `generic-arity`

**What it sees.** Sum of type/const parameters and `where`-clause predicates. Lifetimes have their own lens.

**Default thresholds.** warning `4`, error `7`.

**What "high" means.** A signature with many type parameters and bounds asks the reader to mentally solve a trait-resolution puzzle.

**Refactor hints.**
1. Replace generic parameters with `impl Trait` arguments — the bound disappears from the visible signature.
2. Group co-occurring bounds into a single trait alias (`trait My: A + B + C {}`).
3. If a parameter is always instantiated with one type, drop the genericity.

**References.** plan §2.4.

---

## CLI commands (M1 surface)

### `cargo rustics analyze`

The body of the loop. Walks the workspace, parses every `.rs` file once with `syn`, runs every enabled lens against the AST, and emits a report.

```sh
cargo rustics analyze                    # console reporter (default)
cargo rustics analyze --reporter ai      # YAML-ish for LLM consumption
cargo rustics analyze --reporter json    # machine pipe
cargo rustics analyze --fatal-warnings   # CI gate (non-zero exit on any warning)
cargo rustics analyze --metric cyclomatic-complexity   # only one lens
```

Common options:

| Flag | Meaning |
|------|---------|
| `--root <path>` | Override the analysis root (default: cwd, workspace auto-detected). |
| `--reporter <name>` | `console` \| `json` \| `ai` (default `console`). |
| `--metric <id>` | Run only the named lens. Repeatable / comma-separated. |
| `--exclude-metric <id>` | Skip the named lens. |
| `--fatal-warnings` | Exit non-zero if any warning was reported. |
| `--concurrency <n>` | Worker thread count (default: host CPUs, clamped to 16). |
| `-v`, `--verbose` | DEBUG-level logging. |

### `cargo rustics manual`

Prints the document you are reading. The text is `include_str!`'d at compile time, so install version and printed version cannot diverge (plan §5.4).

```sh
cargo rustics manual
cargo rustics manual | claude -p "explain how to use rustics in one paragraph"
```

### `cargo rustics rules`

Lists every built-in lens with its default thresholds, rationale, and refactor hints — the same metadata `--reporter ai` carries inline on each violation.

```sh
cargo rustics rules                              # all
cargo rustics rules --metric cyclomatic-complexity   # one
```

---

## Reporters

### `console`

Human-readable, lined up. Only the violation set; thresholds and counts at the bottom. Suitable for terminals.

### `json`

Machine pipe. Schema in `schemas/rustics-report.schema.json`. Stable across the 0.x line.

### `ai`

YAML-ish, header-anchored:

```
# rustics ai-report v1
version: 1
generatedAt: 2026-05-08T10:14:00Z
summary:
  filesAnalyzed: 42
  violations: 3
  warnings: 3
  errors: 0
violations:
  - id: a3f1c4e9b2d8f7c5
    file: crates/cargo-rustics/src/runner.rs
    scope: runner::run_metrics
    line: 87
    metric: cyclomatic-complexity
    value: 14
    threshold: 10
    severity: warning
    explain: |
      Cyclomatic Complexity counts the linearly independent paths through a function …
    refactorHints:
      - Extract one branch arm into a helper function …
      - Replace nested `if`/`else` chains with a single `match` …
```

The header `# rustics ai-report v1` is the contract anchor. The version bumps when a field is removed or its semantics change (plan §4.1). Field additions are not breaking.

---

## AI prompt examples

### "What does rustics see in this codebase?"

```sh
cargo rustics analyze --reporter ai > rustics-report.yaml
claude -p "Read rustics-report.yaml. Summarise the top 3 risks in one bullet each."
```

### "Refactor the worst-CC function"

```sh
cargo rustics analyze --reporter ai \
  | claude -p "Pick the highest-CC violation. Apply the first refactor hint. Show me the diff only."
```

### "Why is this metric flagging me?"

```sh
cargo rustics rules --metric cyclomatic-complexity | claude -p "Summarise when I should dismiss this rather than refactor."
```

---

## Stable violation ID

```
id = sha256("<file>|<scope>|<metric>")[..16]
```

Where `<file>` is workspace-relative with `/` separators, `<scope>` is the AST-derived path (`module::Type::method`), and `<metric>` is the kebab-case lens id. The id is **content-stable** — it does not include line numbers — so the AI loop can detect "same violation persisted across refactor" by id alone.

---

## What rustics will *not* tell you (M1)

* `regression` — verifying an AI refactor is not a cosmetic gaming. Coming in M2.
* `coverage gating` — pairing a CC violation with line coverage. M2.
* `unused public API` — the Periphery-style BFS detector. M3.
* Layer 2 metrics — anything that needs `rust-analyzer` (monomorphisation count, true borrow cost). M3.

When you want a stronger signal than M1 can give, write the result of `analyze` to disk, do the refactor, run `analyze` again, and diff by violation id. That is the manual form of `regression`.

---

## Honesty about limits

Every lens carries blind spots. The plan lists them (plan §6.6) and the report's `explain` block names the lens-specific ones. Two general points:

1. **Layer 1 is syn-only.** No type inference, no borrow check. Heuristics (e.g. sealed-aware match) are *structural* — they look at AST shape, not the semantic exhaustiveness check the compiler does. Most of the time the structural shape and the semantic shape agree; when they disagree, you may need to dismiss with a reason.
2. **Metrics are signal, not truth.** A clean report does not mean clean code. A noisy report does not mean bad code. The lens shows you a dimension; the human + agent decide what to do.

---

## Self-application

Rustics runs against itself in CI. Every PR runs `cargo rustics analyze --fatal-warnings` against `crates/rustics` and `crates/cargo-rustics`. We cannot ship a release in which our own code does not pass our own lenses. This is the strongest form of dogfooding: the tool's existence proves the thesis (plan §1.2).
