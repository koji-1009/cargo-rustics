# cargo-rustics manual — for AI agents

> **Operator's manual for AI consumers.** README describes what `cargo-rustics` *is*; this page describes what `cargo-rustics` *does for you* and how to drive it. If you are an AI editing Rust code with an editor-tool harness (Claude Code, Cursor, Codex, Aider, OpenHands), this is your reference.
>
> **How to obtain this:** `cargo rustics manual` — this file is embedded in the binary at compile time. Network is not required.

## The premise — multiple lenses on your own writing

Humans read code and feel things. *"This function is gnarly."* *"This impl is doing too much."* *"I can't tell what scope I'm in."* These reactions are real signals about working-memory load, but they are not reproducible — different reviewers feel them at different thresholds, and an AI doesn't feel them at all.

Decades of software-engineering research has converted those felt reactions into reproducible measurements. Each metric in `cargo-rustics` is one such **lens**: a specific, citation-backed instrument that surfaces a specific kind of "hard to read." None of the lenses is the whole picture. Putting on more than one lens, in succession, is the point.

Most of that catalogue — McCabe 1976, Halstead 1977, CK 1994, Hitz & Montazeri 1995, Martin 1994, Nejmeh 1988, Cognitive Complexity 2018, Drysdale's *Effective Rust* 2024 — never made it into the daily toolbox of working programmers. The cost of *calculating* the number, *interpreting* it, and *acting on it* was each individually expensive for a human reviewer. An AI loop absorbs all three. You compute in a second; the rationale and refactor moves are attached to the violation; the edit is yours to apply. The lenses that the literature catalogued for human reviewers are reachable to you in a way they weren't before.

`cargo-rustics` does not gate. It surfaces. Its core value is letting you, the AI, run the same battery of lenses a careful human reviewer would, then **decide** — refactor, accept, or formally dismiss with a reason. That decision step is first-class.

```
                you propose code
                       │
                       ▼
       ┌──────────────────────────────┐
       │  put on the lenses           │  ← cargo rustics analyze --reporter ai
       │  (reproducible readability)  │
       └──────────────────────────────┘
                       │
              for each violation:
                       │
       ┌───────────────┼───────────────┐
       ▼               ▼               ▼
  REFACTOR         DISMISS         PUNT
  (lens shows      (lens reads     (mark unsure
   real fix)        it but the      and surface
                    structure is    a question)
                    load-bearing)
                       │
                       ▼
       ┌──────────────────────────────┐
       │  verify the lens moved       │  ← cargo rustics regression --reporter ai
       └──────────────────────────────┘
```

The full battery, the selection principles behind it, and the per-lens deviations from each cited source live in [`doc/calibration.md`](calibration.md). This page is the operator's reference for *driving* the loop above.

## TL;DR

```sh
cargo rustics analyze --reporter ai          # see code through every lens
cargo rustics manual                          # read this document
cargo rustics regression --before HEAD~1 --after HEAD   # verify a refactor
```

The AI loop is **manual → analyze → refactor → regression**. `manual` is the entry; `regression` is the exit. `analyze` is the body.

## Why a lens, not a score

A score collapses dimensions. A lens names them. When CC is high, the question is "is the function branchy because of business rules, or because no one extracted the early returns?". When `panic-density` is high, the question is "is this fn defensible against bad input, or is every `.unwrap()` an unproven invariant?". Different lenses, different refactors. A score blurs that.

## Polarity — which way is healthier

Each lens declares a `polarity`:

* `LowerIsBetter` — the default. CC, Cognitive, SLOC, NPATH, Halstead Volume, panic-density, unsafe-block-scope, lifetime-arity, generic-arity, iterator-chain-length, WMC, LCOM4, RFC, efferent-coupling, afferent-coupling.
* `HigherIsBetter` — reserved for future lenses where the desirable direction is up.
* `Informational` — neither direction is universally good. The regression diff still surfaces deltas but doesn't classify them. Currently `instability` only.

Read polarity off the regression diff so you don't accidentally celebrate a metric that drifted the wrong way — an `instability` change is a *change-impact ranking shift*, not a "got better".

## The accept / refactor / dismiss decision

This is the step that distinguishes `cargo-rustics` from a linter. Pick deliberately — don't dismiss to silence, don't refactor to game.

### Refactor when…

The metric points at a real readability problem and the structure is **decomposable without loss of intent**.

For the per-lens first moves, **run `cargo rustics rules --reporter ai`** — that emits the live `REFACTOR_HINTS` array each lens ships with. `cargo rustics analyze --reporter ai` carries the same hints inline on every violation when `--no-auto-explain` is off (the default). The "**Refactor hints.**" prose in the [Lenses](#lenses) section below is a hand-written tour of the same territory but is not generated from the code arrays — when the two disagree, the `rules` output is the source of truth.

### Before you dismiss — engage, don't escape

The most common failure mode in this loop is **dismiss-as-escape**: silencing a violation not because the structure is genuinely load-bearing but because the refactor looks hard enough that dismiss becomes the productive-feeling next move. The signal that you are doing this is the *shape of your dismiss reason itself* — phrases like "the metric is technically right but…", "the threshold is too tight for this idiom", or "splitting wouldn't really help here" are not load-bearing reasons; they are exit phrases.

Three checks before you reach for dismiss:

1. **Have you read the function end-to-end?** Every branch, every condition, every nested helper. If you've only read the signature and the metric value, the dismiss is premature.
2. **Have you tried a specific refactor and rejected it on a concrete structural reason?** "I didn't try" is not a reason. "I tried Extract Method on the deepest branch; the helper became a one-line passthrough that hid the per-case structure the reader needs to see" is a reason. The second sentence names a specific move and what its specific failure was.
3. **Would a careful human reviewer agree the structure is load-bearing?** If you suspect a reviewer would refactor, so should you.

There is one path that looks like dismiss but isn't: **threshold calibration**. When the same kind of violation fires across many sites on the same Rust idiom, the threshold may be wrong for the codebase, not the code. The right move there is `[rustics.metrics.<id>] warning = <n>` in `rustics.toml` — one tracked, operator-audited decision instead of N parallel dismiss entries. Reach for calibration when the same dismiss reason would otherwise repeat 5+ times across the workspace.

### Dismiss when…

The lens reads it correctly but the structure is **load-bearing**: a recursive-descent parser whose grammar mirrors the function shape; a state machine the user calls into; an exhaustive `match` over a sealed-by-design enum; a decoder fan-out where every branch is a real protocol case. Splitting it would hide intent, not clarify it.

A dismiss is a tracked, auditable decision, not a silent disable:

```rust
// rustics:dismiss cognitive-complexity reason="Recursive-descent parser; splitting per-token would hide the grammar"
fn parse(tokens: &mut Tokens) -> Result<Ast, ParseError> { … }
```

Or via the sidecar `.rustics-dismissals.toml` at the workspace root:

```toml
[[dismissals]]
file = "crates/parser/src/lib.rs"
scope = "parser::Parser::dispatch"
metric = "cognitive-complexity"
reason = "Recursive-descent parser; linear structure mirrors the grammar"
by = "claude-opus-4-7"
at = "2026-05-08"
```

`reason` shorter than `min_reason_length` (default 20 chars) is **rejected** — the violation stays live and the report stamps it with the rejection cause. Your dismiss is not silent: if it didn't take, the next pass will tell you why.

Stale entries — dismissals that no longer match any live violation (scope renamed, function deleted, metric dropped below threshold) — appear in the report's `staleDismissals:` block. Treat that as a cleanup candidate: the dismiss is doing nothing now, and leaving it in the file accumulates dead config.

### Punt when…

The lens reads it but you genuinely don't know whether the structure is load-bearing without project context the harness hasn't given you (domain rules, performance constraints, historical bug fixes baked into a function shape).

Punt has **no in-tree syntax** — no comment directive, no TOML key, no field in the JSON report. It is deliberately a natural-language channel between you and the operator, not a tracked artifact. `cargo-rustics` is a tool for both AI and human, and the lens values that anchor your decision don't carry equivalent meaning to the operator the way they do to you; surfacing a `cognitive-complexity: 22` number doesn't transfer the situation. Translate what you saw into the project's own vocabulary, name the load-bearing hypothesis you cannot confirm, and ask in the same channel the harness uses to reach the human. When the answer comes back, route it into refactor or dismiss on the next pass.

### What "refactor to game" looks like

Goodhart's law: when a measure becomes a target, it stops measuring. Three patterns where the metric drops but the code didn't actually get better — self-check before committing:

1. **Half-split**: splitting a function into helpers that can't be named for their *role*, only their *contents* (e.g. `parse_le_or_ge` + `parse_eq_or_ne` for the four two-char operators — the names just describe what each half *contains*, not what either *does*). If you can't name the parts honestly, the responsibility didn't actually break in two; use a `macro_rules!` or data table to keep the logic flat.
2. **Cosmetic split**: complexity moved across more functions while keeping the branching logic. `cargo rustics regression` detects this with `helpersAdded ≥ 3 ∧ slocDelta > 4·helpers ∧ ccReduction < 2·helpers` and emits `cosmeticAnalysis.verdict: likely-cosmetic`. When that fires, **revert your refactor.** Real reduction either removes a dimension (boolean → enum, dispatch table → polymorphism) or consolidates duplicated branches into one parameterised path. Ahead of time, ask: did the total decision count actually drop?
3. **Metric-driven dismiss**: a dismiss whose reason boils down to "I don't want to refactor this". Dismiss is for "the lens is wrong *here*". If the reason would still hold if the metric were 50% lower, the dismiss is genuine; if not, the lens is signal.

`cargo rustics ai-loop` has the long-form treatment with worked examples. Re-read it when a refactor looks too easy.

## High-coverage signal — `complexityJustified`

If `--coverage <path>` is engaged (auto-detected from `target/coverage/lcov.info` when present) the engine attaches a `complexityJustified` block to CC / Cognitive / NPATH / Halstead violations whose scope is well-tested. The current rule is line coverage ≥ 0.95; the branch-coverage rule is reserved for when the lcov branch parser lands.

When the rule fires, `complexityJustified` is a **nested object** carrying the engine's decision:

```yaml
complexityJustified:
  by: line          # line | branch (branch reserved)
  threshold: 0.95   # the cutoff that rule used
  actual: 0.97      # measured coverage ratio
```

The block is absent (not `null`, not `false`) when the rule didn't fire. Reporters pass it through verbatim — JSON, AI / YAML, MD, SARIF.

**Read this as: "the human has already paid the price of branching with tests; refactor at your own risk."** AI loops should generally leave `complexityJustified` violations alone unless the metric is *catastrophically* over threshold (e.g. CC > 2× warning). The AI reporter sorts these to the bottom so they don't compete for token budget.

---

## Lenses

> The catalogue grows every release. Run `cargo rustics rules` for the live list.

### `cyclomatic-complexity` (sealed-aware)

**What it sees.** Linearly independent paths through a function. Branches, loops, `?`, `&&`/`||` each add `+1`. `match` on a non-wildcard arm set contributes `0` because the compiler is checking exhaustiveness for you (sealed-aware adjustment.). `match` *with* a `_` arm contributes `arms - 1`.

**Default thresholds.** warning `10`, error `20`.

**What "high" means.** A function with CC > 10 has more decision points than most readers can hold in working memory in one sitting. Test combinatorics also explode — fully covering N independent branches needs ≥ N test cases.

**Refactor hints.**
1. Extract one branch arm into a helper. The helper carries its own preconditions, the caller stays linear.
2. Replace nested `if`/`else` chains with a `match` on a small enum. The sealed-aware rule then absorbs the branches.
3. Lift early-return guards to the top with `let … else { return … }` so the happy path stays on the spine.
4. If the function is a state machine, name the states: each state is its own small function with a low CC.

**When to dismiss.** Recursive descent parsers, hand-rolled lexers, finite-state machines whose states are intrinsic to the domain. Reason field: write *why splitting hides intent*.

**References.** McCabe 1976.

### `source-lines-of-code` (SLOC)

**What it sees.** Non-blank, non-comment-only lines inside a function body.

**Default thresholds.** warning `60`, error `120`.

**What "high" means.** Long bodies hide what they do. SLOC is the conservative size measure: low SLOC + high CC means dense; high SLOC + low CC means sprawl.

**Refactor hints.**
1. Extract a contiguous block into a named helper. The helper's name is documentation.
2. Lift `let` chains and conversions to the top so the body's shape is visible.
3. Replace long `if`/`else` chains with a `match` (the sealed-aware CC adjustment makes this free at the CC lens).

**References.** —

### `npath-complexity` (off-by-default)

**What it sees.** Number of acyclic execution paths through the function body. Where Cyclomatic Complexity adds 1 per decision point and grows linearly, NPath *multiplies* sequential branches and grows combinatorially: two back-to-back `if-else` blocks score CC=3 but NPath=4; ten compose to CC=11 but NPath=1024. Captures the test-combinatorics cost CC under-counts.

**Default thresholds.** Off-by-default. The lens still emits measurements (so `regression` sees drift), but no warnings fire until you opt in. Self-application shows `r = 0.78` with `cognitive-complexity` and `r = 0.61` with `cyclomatic-complexity` — Cognitive Complexity (Campbell 2018) is the modern refinement of NPath-style path counting and absorbs most of NPath's signal at typical workspace scale. Kept opt-in for the long tail (huge state machines that push CC and Cognitive past their ceilings before NPath). Recommended threshold for opt-in: warning `200`, error `1000` (Nejmeh's 1988 numbers). Apply by adding `[rustics.metrics.npath-complexity]` with `warning = 200` in `rustics.toml`.

**What "high" means.** Past 200 the function exceeds practical exhaustive-testability — Nejmeh's original recommendation. Beyond ~1000 the path space explodes into millions and exhaustive testing is moot.

**Refactor hints.**
1. Pull a sequence of independent decisions into a helper — the helper's NPath grows in isolation; the caller's drops to NP(helper) + 1.
2. Collapse parallel `if-else` chains into a single `match` on a small enum: a 4-arm match scores 4, while four independent if-else blocks compose to 2^4 = 16.
3. A loop with internal branching often factors cleanly: lift the branching out of the loop body into a helper that decides once, then loop over the resulting plan.

**References.** Nejmeh, B. A. (1988). NPATH: a measure of execution path complexity and its applications. Commun. ACM 31(2): 188-200.

### `lifetime-arity`

**What it sees.** Number of explicit lifetime parameters on a function signature. Implicit elision is not counted — that is the point of elision.

**Default thresholds.** warning `3`, error `5`.

**What "high" means.** Each lifetime is one referential constraint the reader has to track. Past three, the signature is a small constraint puzzle that has to be solved before the call.

**Refactor hints.**
1. Push the lifetimes into a struct (`struct Borrow<'a> { ... }`); the function takes `Borrow<'_>` instead.
2. Take ownership where possible — `String` instead of `&'a str`.
3. Many signatures with explicit lifetimes are eligible for elision; try removing them and let rustc tell you.

**References.** —

### `generic-arity`

**What it sees.** Sum of type/const parameters and `where`-clause predicates. Lifetimes have their own lens.

**Default thresholds.** warning `4`, error `7`.

**What "high" means.** A signature with many type parameters and bounds asks the reader to mentally solve a trait-resolution puzzle.

**Refactor hints.**
1. Replace generic parameters with `impl Trait` arguments — the bound disappears from the visible signature.
2. Group co-occurring bounds into a single trait alias (`trait My: A + B + C {}`).
3. If a parameter is always instantiated with one type, drop the genericity.

**References.** —

### `unsafe-block-scope`

**What it sees.** Total inclusive lines of `unsafe { … }` blocks inside a function body. Multiple unsafe blocks sum.

**Default thresholds.** warning `20`, error `50`.

**What "high" means.** Every line inside an `unsafe` block is a soundness obligation. Long unsafe blocks scale the audit surface — five lines you can audit, fifty you cannot.

**Refactor hints.**
1. Pull the `unsafe` block down to the smallest possible expression — the surrounding safe code doesn't need the contract.
2. Wrap the unsafe operation in a small safe wrapper that returns a checked result.
3. Extract repeated unsafe operations into a single audited helper.

**Caveats.** Measures `unsafe { ... }` blocks only, not `unsafe fn` bodies. Self-only — never traverses dependencies (cargo-geiger does that). FFI call count is not implemented.

**References.** —, §6.1, §6.6.

### `panic-density` (unwrap_or-aware)

**What it sees.** Count of `.unwrap()` / `.expect(...)` calls and `panic!` / `unreachable!` / `todo!` / `unimplemented!` / `assert!`-family macros inside a function body. The `unwrap_or-aware` adjustment excludes `.unwrap_or_default()` / `.unwrap_or_else(...)` etc. — they cannot panic by construction.

**Default thresholds.** warning `3`, error `10`.

**What "high" means.** Each panicking site is a runtime crash waiting for the wrong input. A high count says the function is hoping rather than modelling its error cases.

**Refactor hints.**
1. Replace `.unwrap()` on `Option` with `.unwrap_or(default)` or `.ok_or(err)?`.
2. Replace `.expect("...")` on `Result` with `?` so the caller sees the real error.
3. If a panic encodes an invariant the function actually guarantees, document it in a `// SAFETY:` comment and consider `debug_assert!` instead.
4. Wrap repeated panics into one `let-else` guard at the top.

**Caveats.** Test bodies (`#[cfg(test)]`) are not skipped — they contribute to the count alongside production code.

**References.** —, §2.5, §6.6.

### `cognitive-complexity`

**What it sees.** SonarSource 2018 cognitive-complexity. Each control-flow break adds `+1`; structures that *nest* their bodies add an additional bonus equal to the current nesting level. Sequential structures (`else if`, `else`) get the `+1` only.

**Default thresholds.** warning `15`, error `25`.

**What "high" means.** Cognitive Complexity is the cost of *understanding* the code, not testing it. CC counts independent paths; CogC penalises shapes a reader has to mentally unwind: nested control flow, long booleans, labelled breaks crossing scopes.

**Refactor hints.**
1. Each level of nesting compounds — extract the inner-most block into a helper.
2. Replace nested `if`/`else` with a flat `match` on a small enum.
3. Use `?` and `let-else` to lift error paths to the top; the body that follows reads linearly.
4. Long boolean expressions split well into named locals.

**Deviation from SonarSource.** Direct recursion (Sonar charges `+1`) is detected by name match against the enclosing function. Indirect recursion through a call chain is not — that needs cross-function call resolution.

**References.** Campbell 2018.

### `halstead-volume` (off-by-default)

**What it sees.** Halstead 1977 volume `V = N · log2(η)` over a function body. Operators = punctuation, keywords, group delimiters; operands = non-keyword identifiers and literals. Test code is skipped (fixture literals inflate vocabulary without reflecting production complexity).

**Default thresholds.** Off-by-default. The lens still emits measurements (so `regression` sees drift), but no warnings fire until you opt in. Self-application shows `r = 0.84` with `source-lines-of-code` — same "function size" axis measured in a different vocabulary, and Alfadel et al. 2017 documents the CC + SLOC + Halstead Volume mutual correlation. Kept opt-in for the cases where the token-density angle is genuinely useful (review of unfamiliar / refactored / generated code). Recommended threshold for opt-in: warning `1500`, error `3000` (Rust calibration; see below). Apply by adding `[rustics.metrics.halstead-volume]` with `warning = 1500` in `rustics.toml`.

**What "high" means.** Volume captures *information-theoretic* size: a function with many distinct names scores higher than one that reuses the same handful, even at the same line count. Past 1500, the function is doing a lot — for Rust, that often means it's juggling many shapes at once.

**Refactor hints.**
1. Many one-off names (`x_a`, `x_b`, `tmp1`) collapse into a struct or enum; the operand vocabulary shrinks.
2. Long arithmetic / formatting expressions move well into named helpers.
3. Lift repeated literal constants to module-level `const`s.

**Calibration note.** Self-application showed ordinary Rust functions cluster around `700–1500` because of verbose punctuation, so the recommended opt-in defaults are warning `1500`, error `3000` — higher than the textbook `1000` cut-off Halstead's literature usually cites.

**References.** Halstead 1977.

### `wmc`

**What it sees.** Weighted Methods per Class — sum of cyclomatic complexity across every method in a single `impl` block. CK 1994 in its original form. Multiple impls for the same type each emit their own score (one per inherent or trait impl).

**Default thresholds.** warning `50`, error `100`.

**What "high" means.** WMC captures both *width* (many methods) and *depth* (each complex) under one number. A trivial 30-method facade (each method just delegates) scores ~30 — fine. A 5-method coordinator where each branches heavily scores 50+ — the type is doing too much. Empirical studies (Basili et al. 1996, Subramanyam & Krishnan 2003) validate WMC as a defect-density predictor.

**Refactor hints.**
1. Split the impl block by role: separate `impl Foo { /* core */ }` from `impl Foo { /* serde */ }` so each block scores independently.
2. Extract methods that delegate to a helper type; the type's constructor becomes one method and the helper carries the complexity.
3. If the methods share a code structure (e.g. each is a `match` over the same variant), collapse the dispatch into a single method that takes the variant as a parameter.

**References.** Chidamber & Kemerer (1994); Basili, Briand & Melo (1996); Subramanyam & Krishnan (2003).

### `rfc`

**What it sees.** Response For a Class (CK 1994). Per inherent `impl T { … }` block, `RFC = |M| + |R|` where `M` is the methods defined in the block and `R` is the *distinct* methods called by methods of the block. Both `self.foo()` / `other.foo()` (method-call expressions) and `Type::foo(…)` / `Self::foo(…)` (path calls with ≥ 2 segments) contribute to `R`; free-function calls do not (RFC is about method-message dispatch). Methods already in `M` are not double-counted in `R`.

**Default thresholds.** warning `50`, error `100`.

**What "high" means.** A high RFC means even a single entry point on this type pulls in many other methods — the test surface inflates and the reading load when following one call chain inflates with it. CK validated RFC as a defect predictor; Basili et al. (1996) confirmed it ranks classes by maintenance cost across multiple Java codebases.

**Refactor hints.**
1. If most of `R` (the called set) routes through one helper type, depend on that type as a constructor parameter instead of inlining the calls — the response surface narrows.
2. Methods that delegate to many other methods are good candidates for the strategy / template-method shape: pull the varying bits behind a small trait so the impl block calls only one abstract method.
3. If RFC is high because `M` is large (many fn items), see `lcom4` — the impl may be doing several jobs that can split.

**Trait impls are skipped** (same as `lcom4`): the method set is the trait's contract, not a cohesion choice.

**References.** Chidamber & Kemerer (1994); Basili, Briand & Melo (1996).

### `lcom4`

**What it sees.** Lack of Cohesion in Methods, version 4 (Hitz & Montazeri 1995). Number of disjoint connected components in the method graph of an inherent `impl T { … }` block. Methods are nodes; an edge connects two methods when they share at least one `self.<field>` access (including fields written via `Self { x: …, y: … }`), or when one calls the other (`self.other(...)` or `Self::other(...)`). LCOM4 = 1 means the impl is fully cohesive; LCOM4 ≥ 2 means it has independent method clusters that could be split into separate types.

**Default thresholds.** warning `2`, error `5`.

**Rust adaptation.** Trait impls (`impl Trait for T { … }`) are skipped entirely — the method set there is dictated by the trait contract, not a cohesion choice the author can refactor. Within inherent impls, the visitor counts `Self { x: …, y: … }` field initializers as accesses (so a constructor is connected to every accessor) and `Self::method(…)` path calls as call edges (so associated functions and constructors are linked).

**Why LCOM4 (vs CK 1994 LCOM).** The original Chidamber & Kemerer LCOM (defined as `|P| − |Q|` clamped to 0) collapses to 0 for many cohesive *and* incohesive classes — Hitz & Montazeri's component-count fix is the version repeatedly validated as a defect-density predictor (Marinescu 2002).

**Refactor hints.**
1. Group the disjoint clusters into separate types: each cluster becomes a struct that owns the fields its methods touch.
2. If one cluster is a small constructor + helper pair, move it into a free function or an `impl T` block dedicated to that role.
3. Methods that touch *no* fields and aren't called by other methods in the impl form their own singleton component. Consider whether they belong on the type at all — they might be better as free functions.

**References.** Hitz & Montazeri (1995); Marinescu (2002).

### `iterator-chain-length`

**What it sees.** Longest method-call chain on a single value in the function body. Each `.method()` link counts; `let` rebindings break the chain.

**Default thresholds.** warning `6`, error `10`.

**What "high" means.** Method-call chains hide each step's intent. Iterator pipelines naturally chain 3–4 links (`.iter().filter().map().sum()`); past six, the reader is mentally holding a long pipeline of transformations. Naming an intermediate value restores legibility.

**Refactor hints.**
1. Split the chain at the first stateful step (`fold`, `try_fold`, `scan`, `inspect`) — extract the prefix into a named local binding.
2. Long chains often hide an early-return path that wants to be a plain `for` loop. CC drops slightly and the early-return reads explicitly.
3. If the chain ends with `collect()`, see if a `for` loop with `Vec::push` is clearer at the call site.

### `efferent-coupling` (Martin Ce)

**What it sees.** Distinct top-level path roots in the file's `use` statements. `use std::a; use std::b;` is `1`; `use std::a; use serde::b;` is `2`. Internal targets (`crate`, `super`, `self`) and external crates both count.

**Default thresholds.** warning `20`, error `40`.

**What "high" means.** A high Ce means the module reaches outward to many different things — sometimes a legitimate facade, more often unowned responsibility. Pair with Afferent Coupling for the Martin Instability ratio I = Ce / (Ca + Ce).

**Refactor hints.**
1. Pull single-use `use` statements into the function that needs them.
2. If most outgoing edges go to one larger system, extract a small adapter module.
3. Re-exports through a `prelude` collapse many `use` lines into one without changing reach.

### `afferent-coupling` (Martin Ca, cross-file)

**What it sees.** For each `.rs` file (treated as a module identified by `<crate>::<module-path>`), the number of *other* files in this workspace that import from it. External crates (`std`, `serde`, …) do not contribute. Resolution is by longest-prefix module match against the workspace's known crate names (read from `cargo metadata`).

**Default thresholds.** warning `20`, error `40` (mirrors Ce).

**What "high" means.** This module is depended on by many places — modifying its public surface breaks N other files. The metric does not call for a refactor on its own; it ranks change-impact. Note that Rust files don't have Java's "1 public class per file" constraint, so per-file Ca is brittle relative to Martin's original framing; treat the value as a relative change-impact ranking, not as a Pain/Uselessness verdict (see [`doc/calibration.md`](calibration.md) for the per-file granularity caveat).

**Refactor hints.**
1. If many files reach into a single deep symbol, publish a focused re-export at a stable path so the spread of transitive dependents narrows to that surface.
2. Keep the module's public surface trait-shaped so dependents bind to a contract, not a concrete implementation.
3. If the module has both high Ca and high Ce, it is a likely "central hub" — consider splitting it by role.

**References.** Martin (1994).

### `instability` (Martin I, cross-file, informational)

**What it sees.** For each `.rs` file (treated as a module), `I = Ce / (Ce + Ca)` where Ce is the *workspace-internal* outgoing dependency count and Ca is the afferent count. Range `[0, 1]`. `I = 0` → totally stable (depended on; doesn't depend out). `I = 1` → totally unstable (depends out; nothing depends in). Modules with `Ce = Ca = 0` (isolated) are reported as `I = 0`.

**Default thresholds.** None — informational; surfaced as a relative change-impact signal alongside Ca and Ce.

**Why informational.** Martin's `(A, I)` plane and the derived Distance from Main Sequence both depend on Abstractness `A`, which we no longer ship — Rust's lack of a 1-class-per-file constraint makes per-file `A` collapse to 0 for most files (concrete struct + impl + helpers in the same file is the idiomatic shape). Without `A`, the standalone `I` is a relative ranking of "how much does this module live at the leaves vs the stable core" but does not by itself say "this is bad". See [`doc/calibration.md`](calibration.md) for the per-file granularity caveat.

**Reading the value.**
1. `I ≈ 0` (depended on; doesn't depend out) → core foundation modules.
2. `I ≈ 1` (depends out; nothing depends in) → leaf executable / glue modules.
3. Drift in `I` between snapshots is the actionable signal — a foundation module climbing toward `1` means it started leaking outward; a leaf module dropping toward `0` means new dependents found it.

**References.** Martin (1994).

---

## CLI commands

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

Prints the document you are reading. The text is `include_str!`'d at compile time, so install version and printed version cannot diverge.

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

### `cargo rustics regression`

The exit of the AI loop. Diffs two analyze snapshots and classifies every violation: `improved` (gone), `regressed` (new), or `unchanged` (same `id` in both — same problem, same place). A one-word `verdict` summarises the diff: `clean` / `improved` / `regressed` / `mixed` / `unchanged`.

```sh
cargo rustics analyze --reporter json > before.json
# (refactor)
cargo rustics analyze --reporter json > after.json
cargo rustics regression --before before.json --after after.json
cargo rustics regression --before before.json --after after.json --reporter ai
cargo rustics regression --before before.json --after after.json --fatal-regressions   # CI gate
```

The verdict reads top-down for an AI agent:

* `improved` — refactor worked, no regressions; advance.
* `regressed` — refactor broke something; revert or fix.
* `mixed` — partial win, look at the regressed list before advancing.
* `unchanged` — same set of violations; nothing happened (cosmetic refactor).
* `clean` — both snapshots had zero violations.

The cosmetic-refactor detector reads `helpersAdded` / `slocDelta` / `ccReduction` from the diff between snapshots and flags `cosmeticAnalysis.verdict: likely-cosmetic` when the agent moved complexity around without removing it.

---

## Reporters — pick by audience

All five reporters render the same `Report`. Pick by *who reads the output*, not by who runs the command — there is no "primary" reporter and the others are not derived from it.

| Reporter | Audience | Shape |
| --- | --- | --- |
| `--reporter ai` | You — the AI agent in this loop | Token-shaped: auto-explain inlined, priority-sorted, `complexityJustified` sunk to the bottom |
| `--reporter md` | A human reviewer reading the report directly | Markdown sections, rationale + refactor-hint blocks per violated metric, suitable for paste-into-PR |
| `--reporter console` | Human at the terminal | Lined up; thresholds and counts at the bottom |
| `--reporter json` | `jq`, Python, CI gates, programmatic extraction | Schema-stable; validates against `schemas/rustics-report.schema.json` |
| `--reporter sarif` | IDE / CI annotation surfaces (GitHub Code Scanning, GitLab) | SARIF 2.1.0 |

The reporters are **parallel projections** of the same source data, not stages of a pipeline. The metric IDs (16-hex), exact threshold values, and `complexityJustified` sibling fields are bytes the renderers carry verbatim across all five, so a result you read out of `ai` matches the bytes the other four would emit for the same run.

If you have read `--reporter ai` and the destination is now a human or a CI sink, **re-run cargo-rustics with the appropriate reporter flag.** Do not transcribe the ai output by hand: a reconstructed-from-memory copy drifts from the renderer's bytes, undoes the cross-reporter stability `cargo-rustics` is designed to give you, and turns a verbatim-carried metric id into a stable-looking but unstable hex string.

### AI reporter shape

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

The header `# rustics ai-report v1` is the contract anchor. The version bumps when a field is removed or its semantics change. Field additions are not breaking.

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

## How rustics reads your code

Two walkers run, picked per lens by what each lens needs:

1. **AST walker (`ra_ap_syntax`)** — parses each `.rs` file once, no
   workspace context. Function-shape lenses (CC, Cognitive, NPath,
   panic-density, lifetime/generic-arity, unsafe-block-scope,
   iterator-chain-length, SLOC, Halstead Volume, the function-level
   Drysdale lenses) all live here. Per-file, fast.
2. **HIR walker (`ra_ap_hir` / rust-analyzer-as-library)** — loads
   the cargo workspace once per `analyze` run via
   `ra_ap_load_cargo` (macro server in `Sysroot` mode), runs name
   resolution + macro expansion, and walks `Definition`s with
   cross-crate visibility. The `unused` detector lives here today;
   the strong-gain metric lenses (LCOM4, RFC, the Martin trio,
   recursion-aware CC / Cognitive / NPath, sealed-aware
   match-arm-count) are mid-migration. See
   `tmp/hir-default-plan.md` for the per-lens status.

What HIR gives you that the AST walker cannot:

* **Tokens inside macro bodies.** `vec![self.x; n]`,
  `format!("{}", self.field)`, `eprintln!("{}", c.method())` — the
  AST visitor does not parse macro contents, so field accesses or
  method calls hidden inside macro invocations are invisible to
  the AST-only lenses. The HIR backend runs the same macro server
  the compiler uses, so `unused` (today) and the in-flight HIR
  lenses see through.
* **Aliased `self`.** `let s = self; s.field` is invisible to
  LCOM4's field-share rule under the AST walker — only the bare
  keyword `self` is recognised as the receiver. HIR's name
  resolution can follow the binding; the HIR version of LCOM4
  ships in the cohesion-family migration.
* **Resolution-dependent distinctions.** `<Self as
  Trait>::method()` is not currently counted as a self-method
  call by the AST RFC walker. `module::helper()` (free function)
  and `Type::associated_fn()` (method) are indistinguishable at
  the AST level — RFC counts both. HIR resolves each call to its
  canonical `Definition`.

Each lens names its own caveats in the `Caveats.` paragraph
above; this section names the cross-cutting AST-vs-HIR boundary.
Per-run cost: AST walk is sub-second on a 60-file workspace; the
HIR pass adds ~15-20 s of workspace load + macro server + cargo
metadata. First-time install pulls the `ra_ap_*` dep tree
(~170 transitive crates, ~50 s cold build).

---

## Honesty about limits

Every lens carries blind spots. The report's `explain` block names the lens-specific ones; this section names the structural ones. Two general points:

1. **AST lenses are token-shape only.** No type inference, no borrow check. Heuristics (e.g. sealed-aware match before its HIR migration lands) are *structural* — they look at AST shape, not the semantic exhaustiveness check the compiler does. Most of the time the structural shape and the semantic shape agree; when they disagree, you may need to dismiss with a reason. HIR-backed lenses (the `unused` detector today; more lenses landing per `tmp/hir-default-plan.md`) close most of these gaps but still don't see through borrow-checker / MIR-only properties.
2. **Metrics are signal, not truth.** A clean report does not mean clean code. A noisy report does not mean bad code. The lens shows you a dimension; the human + agent decide what to do.

---

## Self-application

Rustics runs against itself in CI. Every PR runs `cargo rustics analyze --fatal-warnings` across the workspace; a release cannot ship while the tool's own code fails the tool's own lenses.

## The operational protocol

For an end-to-end walkthrough with prompt examples, see [`doc/ai-loop.md`](ai-loop.md). The structured reference:

| Step | Command | Notes |
| ---- | ------- | ----- |
| 1. Setup | populate `rustics.toml`; optionally run `cargo llvm-cov --workspace --lcov --output-path target/coverage/lcov.info` | `target/coverage/lcov.info` powers `complexityJustified` |
| 2. Baseline | `cargo rustics analyze --reporter json --snapshot-mode baseline` | Writes `<workspace>/rustics-snapshot.json`. Commit (or save on the CI runner). |
| 3. Read | `cargo rustics analyze --reporter ai --since origin/main --limit 30` | `--since` filters to changed files; `--limit` caps tokens; auto-explain is always on |
| 4. Decide | refactor / dismiss / punt per violation | See [The accept / refactor / dismiss decision](#the-accept--refactor--dismiss-decision) |
| 5. Apply | edit code, add `// rustics:dismiss <metric> reason="…"`, or — if you punted — raise the question to the operator in natural language | `--strict-dismiss` is an audit flag, not a refactor outcome |
| 6. Verify | `cargo rustics regression --before baseline --after HEAD --reporter ai` | Want `verdict: improved` and `cosmeticAnalysis.verdict` ≠ `likely-cosmetic` (i.e. `clean` or `mixed` without the cosmetic signals firing). The verdict enum is `clean / likely-cosmetic / mixed`. |
| 7. Pre-merge | `cargo rustics analyze --strict-dismiss --fatal-warnings` | Ignores dismissals; exits non-zero on any remaining warning |

The same `id` (16 hex chars) reappearing across runs means the previous fix didn't drop the metric. Refactor harder, or formalise as dismiss with a load-bearing reason — there is no third option of "ignore it again."

## Flag map (for reference)

| Goal                                  | Flag                                | Notes                                                                                                  |
| ------------------------------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Pick the AI-shaped report             | `--reporter ai`                     | Mandatory for AI loops                                                                                 |
| Filter to changed files               | `--since <git-ref>`                 | Renames surface as the new path                                                                        |
| Cap output for token budget           | `--limit <n>`                       | Applied after priority sort                                                                            |
| Persist a baseline                    | `--snapshot-mode baseline`          | Writes `<workspace>/rustics-snapshot.json`; commit + CI                                                |
| Persist a local cache                 | `--snapshot-mode cache`             | `target/.rustics-cache/snapshot.json`; gitignored                                                      |
| Skip dismissals (audit)               | `--strict-dismiss`                  | Exposes the raw triage list                                                                            |
| Suppress per-violation explain        | `--no-auto-explain`                 | AI reporter only; other reporters don't auto-explain in the first place                                |
| Inline one lens's rationale anywhere  | `--explain <metric-id>`             | Repeatable; useful for `--reporter md` PR comments                                                     |
| Speed up resolution                   | `--concurrency <n>`                 | Defaults to host CPU count, clamped to 16                                                              |
| Block on warnings                     | `--fatal-warnings`                  | Combine with `--strict-dismiss` for CI                                                                 |
| Run only one lens                     | `--metric <id>`                     | Repeatable / comma-separated                                                                           |
| Skip one lens                         | `--exclude-metric <id>`             | Repeatable                                                                                             |
| Read macro-expanded AST               | `--expanded-macros`                 | Spawns `cargo expand`; slower                                                                          |
| Coverage-aware report                 | `--coverage <path>` (or auto)       | Auto-detects `target/coverage/lcov.info`; `none` disables                                              |
| Lens-pair correlation matrix          | `--statistics`                      | Pearson r on stderr; detects redundant lenses (r > 0.95)                                               |
| Inject metric catalogue once          | `cargo rustics rules --reporter ai` | Feed once into a system prompt                                                                         |
| Verify a refactor                     | `cargo rustics regression`          | Reads `cache` / `baseline` keywords for `--before`                                                     |
| Block on regressions                  | `--fatal-regressions`               | On `regression`; non-zero on any regressed/added                                                       |
| Audit your config                     | `cargo rustics doctor`              | Validates `rustics.toml`; read-only                                                                    |
| Delete unused public-API declarations | `cargo rustics unused --apply`      | In-place deletion. Refuses on a dirty git tree. Filter by kind with `--filter fn,struct,…`             |

## Exit codes

| Code | Meaning                                                | What you do                                                                                         |
| ---- | ------------------------------------------------------ | --------------------------------------------------------------------------------------------------- |
| 0    | Clean (or warnings without `--fatal-warnings`)         | Continue.                                                                                           |
| 1    | Warnings + `--fatal-warnings`, or regressions + `--fatal-regressions` | Either refactor or dismiss with reason; or accept the regression with justification. |
| 64   | Bad CLI args (clap usage error)                        | Re-read your command. `--help` exits 0.                                                             |
| 70   | Internal/runtime error (config parse, IO, `cargo expand`, …) | Stderr names the cause. Config errors include the offending key. If the cause is internal, this is a bug in `cargo-rustics`. |

## What's *not* in the lens battery

Knowing what `cargo-rustics` deliberately doesn't measure is part of the contract:

* **No "code smell" detectors.** No god-object, no feature-envy, no shotgun-surgery heuristics. Those land in noise territory at the false-positive rates either an AST walker or a per-lens HIR walker can support.
* **No automatic fixes for metric violations.** `cargo-rustics` measures and explains. It does not edit your code. (`cargo rustics unused --apply` is the one exception — it removes unreachable public declarations, not metric outliers.) The dismiss channel is *you* writing a comment / TOML entry, not the tool rewriting the source.
* **No ML-derived weights.** Every threshold is documented and overridable. Lens output is reproducible across runs given the same source tree.
* **No cross-PR memory.** The tool doesn't remember "this dismiss was rejected last iteration." Stay session-local.
* **No test-quality lenses.** Coverage is read in only as a complexity-justification signal. Mutation score, assertion density, etc. are out of scope.
* **No inheritance-depth metrics.** DIT and NOC from CK are not provided — Rust has no inheritance and the trait + composition culture makes both signals empty.
* **No per-file abstractness.** Martin's `(A, I)` plane and the derived Distance from Main Sequence are not shipped because per-file `A` collapses to 0 on most Rust files (concrete struct + impl + helpers in the same file is the idiomatic shape). See [`doc/calibration.md`](calibration.md).

## Pointers

* [`README.md`](../README.md) — project overview and install.
* [`AGENTS.md`](../AGENTS.md) — contributor / PR conventions.
* [`doc/ai-loop.md`](ai-loop.md) — narrative walkthrough of one full iteration with sample prompts.
* [`doc/calibration.md`](calibration.md) — citation audit, selection principles, counting-rule deviations.
* `cargo rustics rules --reporter ai` — full rationale + refactor-hint catalogue at runtime.
* [`schemas/rustics-report.schema.json`](../schemas/rustics-report.schema.json) — JSON-reporter output schema (use this if you parse the report yourself).
* [`schemas/rustics-config.schema.json`](../schemas/rustics-config.schema.json) — config schema for `rustics.toml`.
