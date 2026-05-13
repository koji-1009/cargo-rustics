# cargo-rustics — AI loop walkthrough

How to drive `cargo-rustics` from an AI coding agent (Claude, Cursor, Aider, …) end to end.

This is the *operational* doc — it shows the exact commands and prompts. For the lens catalogue and refactor rationale, run `cargo rustics manual`. For repository-level governance, see `AGENTS.md`.

## The loop

```
┌──────────────────────────────────────────────────────────────────┐
│  1. analyze        →  produce a report (rationale + hints)       │
│  2. propose        →  agent reads the report, writes a patch     │
│  3. apply          →  patch is applied                           │
│  4. verify         →  re-run analyze + regression vs the         │
│                       baseline; reject cosmetic refactors        │
└──────────────────────────────────────────────────────────────────┘
```

The contract that makes this reliable:

- **Stable violation IDs** — `sha256(<file>|<scope>|<metric>)[..16]`. The agent can track a specific issue across runs without re-discovering it.
- **Auto-explained AI report** — `--reporter ai` ships rationale + refactor hints inline; no second tool call needed. `--no-auto-explain` strips them back when token budget is tight.
- **`complexityJustified` flag** — a complex function with ≥ 95 % line coverage is marked "earned". The agent should leave it alone.
- **Five-bucket regression diff** — `added` / `removed` / `improved` / `regressed` / `unchanged` lets the agent verify its own work cleanly.
- **Cosmetic detection** — when an agent splits one method into a swarm of one-line helpers without actually reducing branching, the regression report says `cosmeticAnalysis.verdict: likely-cosmetic`.

## Setup (once)

```sh
cargo install cargo-rustics
```

Optional but recommended — pre-load the lens catalogue into the agent's context:

```sh
cargo rustics manual | claude -p "summarise rustics in one sentence per lens"
```

## Step 1 — establish a baseline

Before any agent edits, snapshot the current state:

```sh
cargo rustics analyze --reporter json --snapshot-mode baseline
```

This writes `<workspace>/rustics-snapshot.json`. Commit it (or save it on the CI runner).

## Step 2 — analyze

```sh
cargo rustics analyze --reporter ai
```

Output fragment:

```yaml
# rustics ai-report v1
version: 1
summary:
  filesAnalyzed: 81
  violations: 1
  warnings: 1
  errors: 0
violations:
  - id: a3f1c4e9b2d8f7c5
    file: crates/parser/src/lib.rs
    line: 42
    scope: parser::Parser::parse
    metric: cyclomatic-complexity
    value: 18
    threshold: 10
    severity: warning
    explain: |
      Cyclomatic Complexity counts the linearly independent paths…
    refactorHints:
      - Extract one branch arm into a helper function…
      - Replace nested `if`/`else` chains with a single `match`…
    references:
      - McCabe, T. J. (1976). A Complexity Measure. IEEE TSE.
```

Pipe straight into the agent:

```sh
cargo rustics analyze --reporter ai \
  | claude -p "Refactor the violations following the inline refactor hints. Preserve public API."
```

For PR-scoped runs (`.rs` files changed since `origin/main` only):

```sh
cargo rustics analyze --reporter ai --since origin/main \
  | claude -p "Review the violations on changed files"
```

For token-tight contexts, suppress the explain blocks:

```sh
cargo rustics analyze --reporter ai --no-auto-explain \
  | claude -p "Re-run with --reporter json and read the rationale field if you need full per-violation context"
```

For coverage-aware loops (recommended — flags `complexityJustified`):

```sh
cargo llvm-cov --workspace --lcov --output-path target/coverage/lcov.info
cargo rustics analyze --reporter ai
# `target/coverage/lcov.info` is auto-detected; no --coverage flag needed.
```

## Step 3 — agent applies a patch

The agent edits the source. The `id` it received in Step 2 stays valid as long as `(file, scope, metric)` doesn't change — so it can target a specific violation.

If the agent gets a violation with `complexityJustified:` set, **it should skip that violation**. Tests prove the shape works; refactoring well-tested complex code is the failure mode this signal exists to prevent.

### Anti-patterns: when the metric drops but the code didn't get better

Goodhart's law applies in full force here: *when a measure becomes a target, it ceases to be a good measure*. Three patterns the agent should self-check before committing a refactor — each fires the metric down without making the code easier to read or maintain.

**1. The "half-split"**. Splitting a function into two helpers *purely* to halve a metric, where the resulting halves can't be named for their roles. Example:

```rust
// Before: parse_op tries 7 operators in longest-match order. NPath = 256.
fn parse_op(input) -> Op { /* 7 sequential peek/parse pairs */ }

// "Half-split" anti-pattern — NPath drops, naming has no real meaning:
fn parse_op(input) -> Op {
    if let Some(op) = parse_le_or_ge(input)? { return Ok(op); }
    parse_eq_or_ne(input)
}
fn parse_le_or_ge(input) -> Option<Op> { /* <= and >= */ }
fn parse_eq_or_ne(input) -> Option<Op> { /* == and != */ }

// Honest fix — the responsibility is one thing (longest-match table),
// expressed as one thing. NPath stays low because the visitor doesn't
// walk macro bodies.
fn parse_op(input) -> Op {
    try_op!(input, <=, Op::Le);
    try_op!(input, >=, Op::Ge);
    /* … 5 more rows */
    Err("expected …")
}
```

**Heuristic**: if you split a function into N parts, each part must be named for its *role*, not its *contents*. "Le-or-ge" is contents; "two-char-operator-table" is content too — the responsibility didn't actually break in two. If you can't name the parts honestly, the shape is wrong. Use a `macro_rules!` or data table to keep the logic flat instead.

**2. The "cosmetic split"** (already detected by `cosmeticAnalysis.verdict: likely-cosmetic`, see Step 4). Adding ≥ 3 small helpers, growing total SLOC by more than 4× the helpers, while reducing CC by less than 2× the helpers. This is complexity *moved*, not *removed*. Even when the heuristic doesn't trigger, agents should self-check: did the *number of decision points* actually drop, or just their *distribution*?

**3. The "metric-driven dismiss"**. Adding a `// rustics:dismiss` comment with a load-bearing reason that doesn't survive scrutiny. Dismiss is for "the lens is wrong *here*" (state machines, recursive-descent parsers, exhaustive-by-design dispatch). It is not for "the metric dropped to 9.99 last week and now it's 10.01 again". If the dismiss reason boils down to "I don't want to refactor this", the lens is signal — push back.

**Self-check before committing**:

- Can each helper I introduced be named for its role?
- Did the total decision count drop, or did I move it?
- Did I add a dismiss whose reason would still be true if the metric were 50% lower? (If yes, the dismiss is justified by *intent*, not by the threshold.)

If any answer is "no", revert and try again.

## Step 4 — verify

```sh
cargo rustics regression --before baseline --after HEAD --reporter ai --fatal-regressions
```

`--before baseline` reads `<workspace>/rustics-snapshot.json` directly — no path management. (Use `--before cache` for the local-only `target/.rustics-cache/snapshot.json`.)

Sample output of a *good* refactor:

```yaml
# rustics regression-report v2
version: 2
verdict: improved
diff:
  added: 0
  removed: 1
  improved: 0
  regressed: 0
  unchanged: 0
removedViolations:
  - id: a3f1c4e9b2d8f7c5
    file: crates/parser/src/lib.rs
    scope: parser::Parser::parse
    metric: cyclomatic-complexity
    value: 18
```

Sample output of a **cosmetic refactor** the loop should reject:

```yaml
# rustics regression-report v2
verdict: mixed
diff:
  added: 0
  removed: 0
  improved: 1
  regressed: 0
  unchanged: 0
cosmeticAnalysis:
  signals:
    helpersAdded: 5
    slocDelta: 32
    ccReduction: 4
    clonesAdded: 0
  verdict: likely-cosmetic
```

The `likely-cosmetic` verdict fires when the agent splits one method into ≥ 3 helpers, total SLOC grew by more than 4 × helpers added, AND total CC dropped by less than 2 × helpers added — i.e. the refactor *moved* complexity around without removing it. Reject the patch and ask the agent to try again with a different approach.

## Step 5 — close the loop

If the regression command exits 0 (verdict `clean`, `improved`, or `unchanged`), commit. If it exits 1 under `--fatal-regressions` (verdict `regressed` or `mixed`), reject and re-prompt.

## Loop-closer prompts

### `analyze` → patch

```
You will receive a `# rustics ai-report v1` YAML block. For each violation
in `violations:`:

1. Read `explain:` to understand what shape the lens flags.
2. Choose ONE of the `refactorHints:` and apply it to the function at
   `<file>:<line>` (scope: `<scope>`).
3. Preserve public API. If you must change a public signature, stop and
   surface a question — DO NOT change it silently.
4. If `complexityJustified:` is present, SKIP the violation. The tests
   prove the shape works.
5. After applying patches, output a unified diff.
```

### `regression` → accept / reject

```
You will receive a `# rustics regression-report v2` YAML block.

Accept the patch if:
- verdict is `improved`, `clean`, or `unchanged`.
- OR every entry in `regressed` / `added` has `complexityJustified:` set.

Reject the patch if:
- `cosmeticAnalysis.verdict` is `likely-cosmetic`.
- OR any `regressed` / `added` violation has no `complexityJustified:`
  AND the verdict is `regressed` or `mixed`.

When rejecting, surface the specific ids in `regressedViolations:` /
`addedViolations:` so the agent knows what got worse.
```

### Per-id deep-dive

The full rationale + refactor hints + references for any single violation are already in the JSON / AI report — looking them up is a JSON read, not a second tool call:

```sh
cargo rustics analyze --reporter json \
  | jq '.violations[] | select(.id == "a3f1c4e9b2d8f7c5")'
```

Or if the agent wants the catalogue-level rationale without an active violation:

```sh
cargo rustics rules
```

`cargo rustics rules` lists every shipped lens with its `rationale`, `refactorHints`, and `references` — the same metadata `--reporter ai` carries inline on each violation.

## Dismissals — when the lens is wrong

Some shapes ARE intentional. State machines, parsers with one-arm-per-token, recursive-descent dispatch — they look complex because the *problem* is. Mark these with a dismissal so the lens stops surfacing them on every run:

```rust
// rustics:dismiss cyclomatic-complexity reason="State machine: splitting hides intent"
fn handle_event(&mut self, e: Event) -> Transition {
    match e {
        Event::A => …,
        // …
    }
}
```

Or sidecar `.rustics-dismissals.toml` at the workspace root:

```toml
[[dismissals]]
file = "crates/parser/src/lib.rs"
scope = "parser::Parser::dispatch"
metric = "cyclomatic-complexity"
reason = "Recursive-descent parser; linear structure is intentional"
by = "claude-opus-4-7"
at = "2026-05-08"
```

`reason` must be ≥ 20 characters. Stale dismissals (no longer matching any live violation) are surfaced on the next run as `staleDismissals:` warnings.

## CI shape

```yaml
# .github/workflows/rustics.yml
- run: cargo rustics analyze --fatal-warnings
- run: cargo rustics regression --before baseline --after HEAD --fatal-regressions
```

The first gates *new* violations crossing thresholds; the second gates *regressions* against the committed baseline. Together they cover the complete "did this PR make the codebase worse?" question.

## Tips for the AI agent

- **Re-read `manual` before every iteration** — the embedded copy is short. Free context is worth re-paying.
- **Always pass `--reporter ai`** when consuming output. Other reporters sort by source order; `ai` sorts by actionability (un-justified violations first, justified ones at the bottom).
- **Don't dismiss without a reason ≥ 20 chars.** The reason is the contract that lets a future agent re-evaluate the dismissal.
- **`id` is your memory.** Persist the id of every violation you accept-as-is across runs — when the id reappears unchanged, that's signal.
- **Read `complexityJustified:` before refactoring.** A function with ≥ 95 % line coverage carries the earned-complexity flag; rewriting it risks breaking the tests that justified it in the first place.
- **Use `--snapshot-mode baseline` for the regression baseline**, not a manually-managed JSON path. The CLI handles the location.

## Troubleshooting

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| Same `id` keeps showing up across runs | Refactor didn't actually drop the metric — same `(file, scope, metric)` triple | Inspect `value` vs `threshold` delta; the metric is still over the line. Refactor harder or formalise as dismiss with a load-bearing reason. |
| `dismissalRejected: reason too short` | `require_reason: true` (default) and the reason is under `min_reason_length` (default 20 chars) | Rewrite the reason to ≥ 20 chars and re-run. |
| Stale dismissals appearing in `staleDismissals:` block | A dismissal no longer matches any live violation (scope renamed, function deleted, metric dropped below threshold) | Delete the dead entries from `.rustics-dismissals.toml`. |
| `cosmeticAnalysis.verdict: likely-cosmetic` on a "real" refactor | The diff matches `helpersAdded ≥ 3 ∧ slocDelta > 4·helpers ∧ ccReduction < 2·helpers` | Revert. Real reduction either removes a decision dimension or consolidates branches — it doesn't redistribute them across more functions. |
| `exit 70` with config-parse message | `rustics.toml` invalid or missing required key | Stderr names the offending key; run `cargo rustics doctor` to validate without analyzing. |
| `exit 70` with `cargo expand` error | `--expanded-macros` was passed but `cargo expand` isn't installed or failed on this crate | `cargo install cargo-expand`, or drop `--expanded-macros`. (HIR already sees through `eprintln!` / `format_args!` / `macro_rules!` bodies for the `unused` detector; `--expanded-macros` mostly helps the AST metric lenses when proc-macros generate surprising shape.) |
| AI report missing `explain:` for a violation | `--no-auto-explain` was passed | Re-run without `--no-auto-explain`, or use `--explain <metric-id>` to inline a single lens. |
| AI report shows no violations after edit | `--since <ref>` filtered out the file you changed | Drop `--since`, or rebase so the file shows as changed against the ref. |

## Reference flag map

| Goal | Flag | Notes |
| --- | --- | --- |
| Pick the AI-shaped report | `--reporter ai` | Mandatory for AI loops |
| Filter to changed files | `--since <git-ref>` | Renames surface as the new path |
| Cap output for token budget | `--limit <n>` | Applied after priority sort |
| Persist a baseline | `--snapshot-mode baseline` | `<workspace>/rustics-snapshot.json` (commit + CI) |
| Persist a local cache | `--snapshot-mode cache` | `target/.rustics-cache/snapshot.json` (gitignored) |
| Skip dismissals (audit) | `--strict-dismiss` | Exposes the raw triage list |
| Suppress per-violation explain | `--no-auto-explain` | AI reporter only |
| Inline one lens's rationale | `--explain <metric-id>` | Works on any reporter; repeatable |
| Speed up resolution | `--concurrency <n>` | Defaults to host CPU count, clamped to 16 |
| Block on warnings | `--fatal-warnings` | Combine with `--strict-dismiss` for CI |
| Block on regressions | `--fatal-regressions` | On `regression`; non-zero on any regressed/added |

For the full flag table and exit codes, run `cargo rustics manual` and jump to "Flag map" / "Exit codes".

## What's outside this loop

- **Cross-PR memory** — `cargo-rustics` doesn't track "this dismiss was rejected once; don't propose it again." Stay session-local.
- **Prompt templates per agent** — Claude Code, Cursor, Codex, Aider each have their own conventions. The shell-out pattern in this doc works in all of them; adjust the `claude -p "…"` invocation to your harness's equivalent.
- **Watch mode** — the [`rustics-lsp`](../crates/rustics-lsp) crate covers editor / IDE feedback. The CLI is run-on-demand by design.
- **Type-aware metric lenses** — the `unused` detector resolves names through HIR (`ra_ap_hir`) so it sees through macros and resolves cross-crate consumers, but most metric lenses (CC, Cognitive, NPath, LCOM4, RFC, coupling trio) still walk the AST and would benefit from HIR for the recursion-detection, aliased-self, and re-export-resolution edge cases. Migration is in flight — see `tmp/hir-default-plan.md` for the per-lens status. Borrow-check-aware lenses (would need MIR) are not on the roadmap.
