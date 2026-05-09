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

The AI loop is **manual → analyze → refactor → regression**. `manual` is the entry; `regression` is the exit. `analyze` is the body. All three are wired today.

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

### What "refactor to game" looks like

Goodhart's law: when a measure becomes a target, it stops measuring. Three patterns where the metric drops but the code didn't actually get better — every agent driving rustics should self-check before committing:

1. **Half-split**: splitting a function into helpers that can't be named for their *role*, only their *contents* (e.g. `parse_le_or_ge` + `parse_eq_or_ne` for the four two-char operators — the names just describe what each half *contains*, not what either *does*). If you can't name the parts honestly, the responsibility didn't actually break in two; use a `macro_rules!` or data table to keep the logic flat.
2. **Cosmetic split**: ≥ 3 small helpers, total SLOC up by 4×helpers, CC reduction less than 2×helpers — complexity *moved*, not *removed*. The `regression` command flags this as `cosmeticAnalysis.verdict: likely-cosmetic`. Ahead of time, ask: did the total decision count actually drop?
3. **Metric-driven dismiss**: a dismiss whose reason boils down to "I don't want to refactor this". Dismiss is for "the lens is wrong *here*" (state machines, recursive-descent parsers, exhaustive-by-design dispatch). If the reason would still hold if the metric were 50% lower, the dismiss is genuine; if not, the lens is signal.

`cargo rustics ai-loop` has the long-form treatment with worked examples. Re-read it when a refactor looks too easy.

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

### `npath-complexity`

**What it sees.** Number of acyclic execution paths through the function body. Where Cyclomatic Complexity adds 1 per decision point and grows linearly, NPath *multiplies* sequential branches and grows combinatorially: two back-to-back `if-else` blocks score CC=3 but NPath=4; ten compose to CC=11 but NPath=1024. Captures the test-combinatorics cost CC under-counts.

**Default thresholds.** warning `200`, error `1000`.

**What "high" means.** Past 200 the function exceeds practical exhaustive-testability — Nejmeh's original recommendation. Beyond ~1000 the path space explodes into millions and exhaustive testing is moot.

**Refactor hints.**
1. Pull a sequence of independent decisions into a helper — the helper's NPath grows in isolation; the caller's drops to NP(helper) + 1.
2. Collapse parallel `if-else` chains into a single `match` on a small enum: a 4-arm match scores 4, while four independent if-else blocks compose to 2^4 = 16.
3. A loop with internal branching often factors cleanly: lift the branching out of the loop body into a helper that decides once, then loop over the resulting plan.

**References.** Nejmeh, B. A. (1988). NPATH: a measure of execution path complexity and its applications. Commun. ACM 31(2): 188-200.

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

### `clone-density`

**What it sees.** Count of `.clone()`, `.to_owned()`, `.to_string()` calls inside a function body. Raw count, not a semantic judgement.

**Default thresholds.** warning `5`, error `10`.

**What "high" means.** A function with high clone density is usually escaping the borrow checker by allocating. Sometimes that's the right answer; often it's the path of least resistance.

**Refactor hints.**
1. Borrow instead of clone — `&str` instead of `String`, `&[T]` instead of `Vec<T>`.
2. If data outlives the function, take ownership once at the top and pass references down.
3. `Rc::clone` and `Arc::clone` are reference bumps, not allocations — dismiss with reason.
4. When several clones target the same value, hoist `.clone()` to a single local.

**Caveat.** No semantic discrimination — `String::clone` (allocation) and `Rc::clone` (refcount bump) count the same. Cheap literal clones (`"foo".to_string()`) also count.

**References.** plan §2.4, plan §6.6.

### `unsafe-block-scope`

**What it sees.** Total inclusive lines of `unsafe { … }` blocks inside a function body. Multiple unsafe blocks sum.

**Default thresholds.** warning `20`, error `50`.

**What "high" means.** Every line inside an `unsafe` block is a soundness obligation. Long unsafe blocks scale the audit surface — five lines you can audit, fifty you cannot.

**Refactor hints.**
1. Pull the `unsafe` block down to the smallest possible expression — the surrounding safe code doesn't need the contract.
2. Wrap the unsafe operation in a small safe wrapper that returns a checked result.
3. Extract repeated unsafe operations into a single audited helper.

**Caveats.** M1 measures only `unsafe { ... }` blocks (not `unsafe fn` bodies). Self-only — never traverses dependencies (cargo-geiger does that). FFI call count is M2.

**References.** plan §2.4, §6.1, §6.6.

### `panic-density` (unwrap_or-aware)

**What it sees.** Count of `.unwrap()` / `.expect(...)` calls and `panic!` / `unreachable!` / `todo!` / `unimplemented!` / `assert!`-family macros inside a function body. The `unwrap_or-aware` adjustment (plan §2.5) excludes `.unwrap_or_default()` / `.unwrap_or_else(...)` etc. — they cannot panic by construction.

**Default thresholds.** warning `3`, error `10`.

**What "high" means.** Each panicking site is a runtime crash waiting for the wrong input. A high count says the function is hoping rather than modelling its error cases.

**Refactor hints.**
1. Replace `.unwrap()` on `Option` with `.unwrap_or(default)` or `.ok_or(err)?`.
2. Replace `.expect("...")` on `Result` with `?` so the caller sees the real error.
3. If a panic encodes an invariant the function actually guarantees, document it in a `// SAFETY:` comment and consider `debug_assert!` instead.
4. Wrap repeated panics into one `let-else` guard at the top.

**Caveats.** Production-vs-test mode (skip `#[cfg(test)]` bodies) is M2.

**References.** plan §2.4, §2.5, §6.6.

### `result-chain-depth`

**What it sees.** Longest contiguous chain of `?` operators inside a single expression. `a()?.b()?.c()?` is depth 3. Sequential `?`s across separate statements each contribute depth 1.

**Default thresholds.** warning `6`, error `10`.

**What "high" means.** Each `?` is an early-return point. Inference makes them mechanical, so the threshold is generous — past 6 links a reader still has to track which `?` corresponds to which fallible step.

**Refactor hints.**
1. Break the chain into named locals: `let x = a()?; let y = x.b()?; …`. Each step gets a name; depth resets.
2. If most of the chain is `.method()?`, consider whether the underlying `.method()` should return the unwrapped type already.

**Caveats.** Hand-rolled `match Result { Ok => …, Err => … }` ladders are not measured at M1 — that adjustment needs type info and lands in M2.

**References.** plan §2.4, §2.5.

### `await-depth`

**What it sees.** Longest chain of `.await` operators inside a single expression. `a().await.b().await` is depth 2. Sequential `.await`s across separate statements each contribute depth 1 (plan §6.1 — "sequential はカウント外").

**Default thresholds.** warning `3`, error `5`.

**What "high" means.** Nested awaits compose several async operations into one sequenced computation. Past three links the chain is hard to reason about for cancellation and error propagation.

**Refactor hints.**
1. Pull each `.await` into its own `let` binding.
2. If awaits run a pipeline, use an explicit combinator (`tokio::try_join!`, `futures::join!`) so the parallel structure is visible.
3. `await?` is shorthand for two operations — splitting them often clarifies the error handling.

**References.** plan §2.4, §6.1.

### `cognitive-complexity`

**What it sees.** SonarSource 2018 cognitive-complexity. Each control-flow break adds `+1`; structures that *nest* their bodies add an additional bonus equal to the current nesting level. Sequential structures (`else if`, `else`) get the `+1` only.

**Default thresholds.** warning `15`, error `25`.

**What "high" means.** Cognitive Complexity is the cost of *understanding* the code, not testing it. CC counts independent paths; CogC penalises shapes a reader has to mentally unwind: nested control flow, long booleans, labelled breaks crossing scopes.

**Refactor hints.**
1. Each level of nesting compounds — extract the inner-most block into a helper.
2. Replace nested `if`/`else` with a flat `match` on a small enum.
3. Use `?` and `let-else` to lift error paths to the top; the body that follows reads linearly.
4. Long boolean expressions split well into named locals.

**M1 deviation from SonarSource.** Direct recursion (Sonar charges `+1`) is not detected at Layer 1 — that needs the enclosing function's name and call resolution.

**References.** Campbell 2018; plan §6.1.

### `halstead-volume`

**What it sees.** Halstead 1977 volume `V = N · log2(η)` over a function body. Operators = punctuation, keywords, group delimiters; operands = non-keyword identifiers and literals. Test code is skipped (fixture literals inflate vocabulary without reflecting production complexity).

**Default thresholds.** warning `1500`, error `3000`.

**What "high" means.** Volume captures *information-theoretic* size: a function with many distinct names scores higher than one that reuses the same handful, even at the same line count. Past 1500, the function is doing a lot — for Rust, that often means it's juggling many shapes at once.

**Refactor hints.**
1. Many one-off names (`x_a`, `x_b`, `tmp1`) collapse into a struct or enum; the operand vocabulary shrinks.
2. Long arithmetic / formatting expressions move well into named helpers.
3. Lift repeated literal constants to module-level `const`s.

**Calibration note.** Plan §8 listed 1000 as the warning threshold. Self-application showed ordinary Rust functions cluster ~700–1500 because of verbose punctuation, so M1 raises the warning to 1500 and the error to 3000 (plan §2.3 — "M1 で signal を確認").

**References.** Halstead 1977; plan §2.3, §6.1.

### `impl-trait-fanout` (informational)

**What it sees.** Count of `impl Trait` occurrences in a function signature (arguments + return type, recursing through references / parens / generics). Informational at M1 — no thresholds, the value feeds the `rustContext` block (plan §4.3) once `regression` lands in M2.

**Refactor hints.**
1. If callers need to name the type, prefer a concrete type or alias.
2. When `impl Trait` is genuinely hiding the type (RPIT for async / iterators), keep it.

### `dyn-density` (informational)

**What it sees.** Count of `dyn Trait` occurrences in a function signature: `&dyn`, `Box<dyn>`, `Vec<Box<dyn ...>>`. Informational at M1.

**Refactor hints.**
1. If only a small set of types implements the trait, prefer a generic parameter or enum.
2. Inside hot loops, `Box<dyn T>` → `T: Trait` removes per-call indirection.

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

### `impl-length` (informational)

**What it sees.** Total physical lines of an `impl` block (open brace to close brace, inclusive).

**Default thresholds.** None — informational only.

**Why informational.** Dogfooding showed `r = 0.866` between `impl-length` and `wmc` (CK-defined Weighted Methods per Class). Two metrics of the same impl-block axis would double-count when an AI agent reads both. `wmc` is the citation-backed gate; `impl-length` travels along as raw context (raw line count, no judgment).

**Refactor hints.**
1. If the length is from many short methods, see `wmc` (complexity-weighted view).
2. If the length is from a few huge methods, it's a function-level lens problem (CC, SLOC).

### `trait-method-count`

**What it sees.** Method count in a `trait` definition (required + provided).

**Default thresholds.** warning `15`, error `30`.

**What "high" means.** A trait with many methods imposes a heavy contract on every implementor.

**Refactor hints.**
1. Split into a hierarchy: `trait Read`, `trait Write`, `trait ReadWrite: Read + Write {}`.
2. Move always-defaulted helpers into a separate `*Ext` trait.

### `trait-default-impl-ratio` (informational)

**What it sees.** Ratio of methods with default bodies over total methods, range `[0.0, 1.0]`. Informational at M1 — feeds the `rustContext` block in M2.

### `macro-rules-arm-count`

**What it sees.** Number of arms in a `macro_rules!` definition (counted by `=>` token pairs in the body).

**Default thresholds.** warning `8`, error `15`.

**What "high" means.** A `macro_rules!` with many arms is the `match` of macro-land. Past 8 the order-dependence between rules becomes hard to keep straight.

**Refactor hints.**
1. Push category dispatch into a helper macro called from the main macro's arms.
2. Past a dozen arms, a procedural macro (`#[proc_macro]`) is usually the right tool.
3. Defensive catch-all arms (`($($any:tt)*) => {}`) sometimes outlive their purpose — check.

### `match-arm-count` (sealed-aware)

**What it sees.** Maximum number of arms across every `match` expression inside the function body — but only when the match has a *catch-all* arm (`_ =>` or `name =>`). Exhaustive `match Enum {…}` with no wildcard is the sealed-aware case (plan §2.5): the compiler is checking exhaustiveness, so the lens contributes 0.

**Default thresholds.** warning `7`, error `12`.

**What "high" means.** A non-exhaustive match with many arms is a switch table written by hand — the reader holds each pattern in working memory while scanning for the one that applies. Sealed enum dispatch is exempted because adding a variant forces every match site to update at compile time, so there's no missed-case risk to flag.

**Refactor hints.**
1. Group arm clusters into a helper enum: `enum Action { File(FileOp), Net(NetOp) }` then match those.
2. Use guard clauses on early arms (`0..10 if x % 2 == 0 => …`) to collapse repetitive conditions.
3. Replace string-keyed dispatch with a `HashMap<&'static str, fn(...)>` lookup at the call site.
4. Wide matches inside `impl Trait for T` can usually be split — each variant's arm becomes its own helper method.

**When to dismiss.** Exhaustive dispatch over an open-ended external enum (`syn::Item`, `serde_json::Value`) where each arm reads a different field — refactoring into a data table loses readability without reducing branching.

### `proc-macro-presence`

**What it sees.** Functions decorated with a single-segment proc-macro attribute (e.g. `#[tokio::main]` is multi-segment and ignored; `#[my_macro]` is counted). Layer 1 sees only the syntactic attribute — what the macro expands into is invisible until `--expanded-macros`.

**Default thresholds.** warning `1`, error `3` (informational; thresholds gate "is this function shaped by a heavy macro?" not "is the macro itself bad").

**What "high" means.** Each proc-macro re-shapes the function into something the metric pipeline sees only as a signature. CC, SLOC, and the borrow profile all reflect the un-expanded form. A function with multiple proc-macro attributes is hiding most of its real shape from every other lens.

**Refactor hints.**
1. Run `cargo rustics analyze --expanded-macros` to see what the macro actually generated.
2. If the proc-macro is yours, audit whether the expansion is shorter than the un-expanded form — sometimes a proc-macro introduces more complexity than it removes.
3. Stack two proc-macros (`#[serde(...)] #[validate(...)]`) only when the expansions compose; otherwise interleaved expansion makes downstream debugging miserable.

### `borrow-profile-owned` (informational)

**What it sees.** Count of function parameters taken by value (`fn f(x: T)`). The `self` receiver is excluded — it shows up in `impl-method-count` already.

**Default thresholds.** Informational. The signal lives in the ratio with `borrow-profile-borrowed` and `borrow-profile-mut` (read via the `borrowProfile:` sub-block on the `rustContext` of each violation), not per-lens thresholds.

**What "high" means.** A function that takes 4 owned parameters is paying for 4 moves. Many owned parameters that flow into a struct constructor often want to be the constructor's `Self` directly.

### `borrow-profile-borrowed` (informational)

**What it sees.** Count of function parameters taken by immutable reference (`fn f(x: &T)`). Same exclusions as `borrow-profile-owned`.

**Default thresholds.** Informational — feeds the `rustContext.borrowProfile.borrowed` value.

**What "high" means.** Many immutable borrows are usually fine; the lens exists so `cargo rustics regression` can see the ratio shifting. A function that accumulates immutable borrows over time without picking up mutable ones is gathering read-only context — often a sign the next refactor wants a context struct.

### `borrow-profile-mut` (informational)

**What it sees.** Count of function parameters taken by mutable reference (`fn f(x: &mut T)`). Same exclusions as `borrow-profile-owned`.

**Default thresholds.** Informational — feeds the `rustContext.borrowProfile.mutBorrowed` value.

**What "high" means.** Many `&mut` parameters hint at a god-method that wants to be a method on a single receiver type. The borrow checker will fight harder with each one; an AI agent reading high `mutBorrowed` should propose either consolidation into `&mut self` or interior mutability via `RefCell` (when the constraints allow it).

### `closure-arity`

**What it sees.** Count of inline closure expressions in a function body — every `|...| { ... }` and `move |...| ...` literal.

**Default thresholds.** warning `6`, error `12`.

**What "high" means.** Iterator pipelines naturally hit 3–5 closures. Past six, the function reads as a chain of small lambdas with their own captures rather than a sequence of statements. Reading it requires simulating each closure's body for every call site.

**Refactor hints.**
1. Extract a closure that captures more than one local into a named local function. Captures become arguments and the body reads linearly.
2. Long iterator chains often split at the first stateful step (`fold`, `try_fold`, `scan`); the post-split portion becomes a plain `for` loop without losing brevity.
3. Closures whose bodies are themselves multi-statement blocks usually want to be functions — `|x| { let y = …; let z = …; … }` is a function in disguise.

### `format-density`

**What it sees.** Count of `format!`-class macro invocations per function body: `format!`, `println!`, `eprintln!`, `print!`, `eprint!`, `write!`, `writeln!`.

**Default thresholds.** warning `5`, error `10`.

**What "high" means.** Each format-class macro builds a `String` through the formatting machinery — fine in setup / display code, expensive in hot loops. Companion to `clone-density`: format calls are *another* allocation site that escapes the borrow story.

**Refactor hints.**
1. Pre-format strings outside a hot loop into a `&str` and reuse them inside.
2. Replace `format!` + `push_str` chains with `write!` on a re-used `String` / `Vec<u8>` buffer.
3. If most calls are `println!` / `eprintln!`, consider whether the function should return a value the caller logs at one site instead.

### `iterator-chain-length`

**What it sees.** Longest method-call chain on a single value in the function body. Each `.method()` link counts; `let` rebindings break the chain.

**Default thresholds.** warning `6`, error `10`.

**What "high" means.** Method-call chains hide each step's intent. Iterator pipelines naturally chain 3–4 links (`.iter().filter().map().sum()`); past six, the reader is mentally holding a long pipeline of transformations. Naming an intermediate value restores legibility.

**Refactor hints.**
1. Split the chain at the first stateful step (`fold`, `try_fold`, `scan`, `inspect`) — extract the prefix into a named local binding.
2. Long chains often hide an early-return path that wants to be a plain `for` loop. CC drops slightly and the early-return reads explicitly.
3. If the chain ends with `collect()`, see if a `for` loop with `Vec::push` is clearer at the call site.

### `boxed-allocation-density`

**What it sees.** Count of `Box::new`, `Box::pin`, and `Box::leak` calls in a function body. The constructor literal `Box::<T>::new` matches.

**Default thresholds.** warning `4`, error `8`.

**What "high" means.** Heap allocations in Rust are explicit; a function that boxes things four times is paying four allocations. Trait objects, `Pin`-required futures, and recursive types are legitimate uses; clusters past four usually want extraction into a typed builder or a refactor toward references.

**Refactor hints.**
1. If the boxes hold trait objects, see whether one generic `T: Trait` would work — generics are usually monomorphised away.
2. `Box::pin` for futures is a sign the function is trying to be its own executor; consider an `async fn` that returns the `impl Future` directly.
3. Recursive types (`Box<Self>`) past two-deep usually want a flat representation (`Vec<Node>` with index handles).

### `early-return-density`

**What it sees.** Count of explicit `return ...;` keyword expressions inside a function body. The implicit trailing tail expression is *not* counted (it's a different shape — see also `cyclomatic-complexity`).

**Default thresholds.** warning `5`, error `10`.

**What "high" means.** Two or three early returns guard preconditions; past five, the function is usually hiding control flow that wants to live in an explicit `match` or be split across helpers.

**Refactor hints.**
1. Convert a chain of `if cond { return x; }` guards into an explicit `match` whose arms compute the result.
2. If returns split into two clusters (precondition rejection vs. business-logic shortcut), the second cluster is often a helper function in disguise.
3. Returns inside a `loop` / `for` are different — they are flow control, not guards. Refactoring those tends to make the code worse.

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

**What "high" means.** This module is depended on by many places — modifying its public surface breaks N other files. High Ca paired with high abstractness `A` is healthy ("stable + abstract" sits on Martin's *main sequence*); high Ca with low A means the module is a concrete bottleneck (the "rigid hub" anti-pattern). The metric does not call for a refactor on its own; it ranks change-impact.

**Refactor hints.**
1. If many files reach into a single deep symbol, publish a focused re-export at a stable path so the spread of transitive dependents narrows to that surface.
2. Pair with `abstractness` (A): a high-Ca module wants a trait-shaped public surface so dependents bind to a contract, not a concrete implementation.
3. If the module has both high Ca and high Ce, it is a likely "central hub" — consider splitting it by role.

**References.** Martin (1994). Plan §6.3.

### `instability` (Martin I, cross-file, informational)

**What it sees.** For each `.rs` file (treated as a module), `I = Ce / (Ce + Ca)` where Ce is the *workspace-internal* outgoing dependency count and Ca is the afferent count. Range `[0, 1]`. `I = 0` → totally stable (depended on; doesn't depend out). `I = 1` → totally unstable (depends out; nothing depends in). Modules with `Ce = Ca = 0` (isolated) are reported as `I = 0`.

**Default thresholds.** None — informational. The actionable derived metric is Distance from Main Sequence (`D = |A + I − 1|`).

**Why informational alone.** A high I is fine for a leaf module with no inbound dependents (it lives at the top of the dependency tree by design). A low I is fine for a stable foundation module (Ce = 0 is the goal there). Without pairing with abstractness `A`, a single I value cannot say "this is bad". The pair `(A, I)` is what Martin's *main sequence* (the line `A + I = 1`) evaluates.

**Reading the value.**
1. `I ≈ 0` & `A ≈ 1` (depended on, abstract) → on the main sequence at the "stable abstraction" end. This is what core trait modules look like.
2. `I ≈ 1` & `A ≈ 0` (depends out, concrete) → on the main sequence at the "unstable concretion" end. This is what leaf executable / glue modules look like.
3. `I ≈ 0` & `A ≈ 0` → "zone of pain": rigid concrete bottleneck, hard to change. The Distance lens flags this.
4. `I ≈ 1` & `A ≈ 1` → "zone of uselessness": abstract but nothing uses it. The Distance lens flags this too.

**References.** Martin (1994). Plan §6.3.

### `trait-impl-fanout` (cross-file)

**What it sees.** For each type name, the number of `impl` blocks across the workspace that target it (both inherent `impl Foo { … }` and trait `impl Trait for Foo` count).

**Default thresholds.** warning `8`, error `16`.

**What "high" means.** Many distinct impls on one type often signal that the type is doing several jobs at once — separate inherent blocks each owning a role, plus trait impls for serialisation, display, conversion, and so on. The fanout measurement triangulates "this type accreted responsibilities" before any single impl block looks unreasonable.

**Refactor hints.**
1. If the impls split cleanly by role (serde / display / domain logic), extract the marginal ones into a wrapper type and impl on that.
2. Trait impls that only forward to one method are good candidates to move to a `*Ext` blanket trait.
3. Multiple inherent impls (`impl Foo { ... }` repeated) can usually collapse into one block — splitting them is stylistic and the fanout count exaggerates the spread.

### `abstractness` (Martin A, informational)

**What it sees.** Fraction of type-defining items that are `trait`s: `trait_count / (trait + struct + enum + union + type_alias) count`. Range `[0.0, 1.0]`. Informational at M1 — pairs with Instability (M2) to compute Distance from Main Sequence `D = |A + I − 1|`.

**Refactor hints.**
1. A module mixing many traits with many concrete types splits well into `*_traits` + `*_impl`.
2. Sealed-trait files legitimately sit lower — that pattern is fine.

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

A nuanced cosmetic-refactor detector (helpersAdded / slocDelta / ccReduction signals from plan §4.5) lands when the snapshot format grows to carry all per-scope measurements (M2 follow-up). Today the verdict reads from the violation-id diff alone.

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
