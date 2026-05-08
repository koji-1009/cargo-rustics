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
  | claude -p "Use cargo rustics explain <id> if you need rationale"
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

```sh
cargo rustics explain a3f1c4e9b2d8f7c5 --snapshot report.json
```

Returns the full rationale + refactor hints + references for a single id, even if `--no-auto-explain` was used during analyze.

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
