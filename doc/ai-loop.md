# cargo-rustics AI loop walkthrough

A concrete, end-to-end example of using cargo-rustics from an AI agent's perspective. Setup → analyze → refactor → verify.

## 0. Setup (once per machine)

```sh
cargo install cargo-rustics
```

That is enough to run every command in this document. No additional configuration is required for the M1 surface.

## 1. Read the manual once

```sh
cargo rustics manual
```

The manual is embedded in the binary (`include_str!` at compile time). Install version and printed version cannot drift apart. Pipe it into your context window:

```sh
cargo rustics manual | claude -p "summarise rustics in one sentence per command"
```

## 2. Probe the project

```sh
cd path/to/rust/project
cargo rustics analyze --reporter ai > rustics-report.yaml
```

Read `rustics-report.yaml` — every violation carries a stable `id`, a file/scope/line, the offending value, the threshold, the rationale, and refactor hints. Field names are stable across 0.x.

## 3. Pick one violation

A common heuristic is "highest severity, then highest value":

```sh
cargo rustics analyze --reporter ai \
  | claude -p '
      You will refactor exactly one violation.
      Pick the one with severity=error first, severity=warning otherwise.
      Among ties, pick the highest value-over-threshold ratio.
      Output: file, scope, line, metric, value, threshold, the chosen refactor hint.
  '
```

## 4. Apply the refactor

Use the chosen hint. In practice the hint is one of: extract a helper, replace `if`/`else` with `match`, lift early-return guards, split into a state machine. The hints in `rustics-report.yaml` are concrete enough to act on directly.

## 5. Verify (manual `regression` until M2 ships it)

Today, the `regression` subcommand is on the M2 roadmap. The manual form is:

```sh
# before refactor
cargo rustics analyze --reporter json > before.json

# refactor

cargo rustics analyze --reporter json > after.json
diff <(jq -S . before.json) <(jq -S . after.json) | less
```

Look for two patterns:

1. **Improved.** The violation's `id` is gone in `after.json`, no new ids appeared.
2. **Cosmetic.** The violation's `id` is gone but several new low-value violations appear (helper functions with their own CC overhead). Plan §4.5 — when M2 lands, `cargo rustics regression` will name this `verdict: likely-cosmetic` automatically.

## 6. Loop

Pick the next violation. Keep going until the analyze run is clean enough that further refactors stop paying.

---

## Tips for the AI agent

- **Re-read `manual` before every iteration** — the embedded copy is short. Free context is worth re-paying.
- **Always pass `--reporter ai`** when you're consuming the output. Other reporters sort by source order; `ai` sorts by actionability.
- **Don't dismiss a violation without a reason in the comment.** The reason is the contract that lets a future agent re-evaluate the dismissal.
- **`stable id` is your memory.** Persist the id of every violation you accept-as-is across runs — when the id reappears unchanged, that is *signal*, not noise.
