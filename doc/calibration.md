# Calibration

rustics' lens battery is anchored to published sources. This page is the
audit trail for that anchoring: what's selected and why, where the
implementation departs from the source's literal definition, where the
default threshold differs from the textbook value, and what was
deliberately dropped.

Threshold *numbers* (e.g. CC warn 10, Halstead warn 1500) follow the
cited sources where the literature gives one; calibrated deviations are
documented per-lens with a "Calibration note" pinning the change to
self-application data on this codebase. What can also differ is *what
is counted* — those deviations are listed below with their
justification.

## Selection principles

- **Each lens cites a published source.** Either CS literature
  (McCabe, Halstead, Chidamber & Kemerer, Hitz & Montazeri, Nejmeh,
  Martin, Campbell / SonarSource) or community-formal sources
  (Effective Rust, Rust API Guidelines). "Something I noticed" is
  not a lens — see AGENTS.md.
- **Multicollinearity is checked.** Pairs with `|r| ≥ 0.95` on
  self-application get dropped. Distance from Main Sequence was
  removed under this rule when it correlated `r = −0.994` with
  Instability (the implementation was shipped, then deleted; the
  removal is the canonical example of self-application calibration
  acting on the catalogue).
- **One lens, one signal.** Lenses that derive purely from
  already-shipped lenses (Halstead Difficulty/Effort,
  Maintainability Index = `CC + V + LOC`) add no orthogonal signal
  and are absent.
- **Idiom-misaligned lenses are excluded, not opt-in.** DIT and NOC
  describe inheritance depth/breadth; Rust has no inheritance and
  the trait + composition culture makes both signals empty.
- **Off-by-default / informational when overlap or
  assumption-misfit is structural.** `abstractness` ships as
  informational because the actionable derived metric (Distance
  from Main Sequence) was dropped under multicollinearity, but
  the raw `A` value is kept so the AI report still surfaces it.

## Selected lenses

### Function-level (CS literature)

| Lens | Source |
| --- | --- |
| `cyclomatic-complexity` | McCabe 1976 |
| `cognitive-complexity` | Campbell / SonarSource white paper 2018 — *industry source, not peer-reviewed* |
| `halstead-volume` | Halstead 1977 |
| `npath-complexity` | Nejmeh 1988 |

### Function-level (Rust-idiom — community-formal)

| Lens | Source |
| --- | --- |
| `panic-density` | Effective Rust 2nd ed. §6.1, §6.6 |
| `unsafe-block-scope` | Effective Rust 2nd ed. §6.1, §6.6 |
| `result-chain-depth` | Effective Rust 2nd ed. §6.1 |
| `await-depth` | Effective Rust 2nd ed. §6 (async chapter) |
| `clone-density` | Effective Rust 2nd ed. (ownership chapter) |
| `lifetime-arity` | Effective Rust 2nd ed. (lifetimes chapter) |
| `generic-arity` | Effective Rust 2nd ed. (generics chapter) |
| `closure-arity` | Effective Rust 2nd ed. (closures chapter) |
| `format-density` | Effective Rust 2nd ed. |
| `iterator-chain-length` | Effective Rust 2nd ed. (iterators chapter) |
| `boxed-allocation-density` | Effective Rust 2nd ed. (heap allocation) |
| `source-lines-of-code` | Boehm 1981 (informally; widespread industry convention) |

### Class / impl-block level (CS literature)

| Lens | Source |
| --- | --- |
| `lcom4` | Hitz & Montazeri 1995; Marinescu 2002 |
| `wmc` (Weighted Methods per Class) | Chidamber & Kemerer 1994; Basili, Briand & Melo 1996; Subramanyam & Krishnan 2003 |
| `rfc` (Response For a Class) | Chidamber & Kemerer 1994; Basili, Briand & Melo 1996 |
| `trait-method-count` | (community convention; no specific paper) |

### Cross-file / module-level (CS literature)

| Lens | Source |
| --- | --- |
| `efferent-coupling` (per-file Ce) | Martin 1994 |
| `afferent-coupling` (cross-file Ca) | Martin 1994 |
| `instability` (`I = Ce / (Ca + Ce)`, informational) | Martin 1994 |
| `abstractness` (Martin A, informational) | Martin 1994 |
| `trait-impl-fanout` (cross-file) | (community convention; see audit gaps) |

### Rust-shape probes (informational)

| Lens | Source |
| --- | --- |
| `macro_rules-arm-count` | Effective Rust 2nd ed. (macros chapter) — *audit pending* |

Default thresholds and per-lens descriptions live in [`doc/manual.md`](manual.md)
("Lenses"). Full bibliographic citations are exposed by each lens's
`references` getter and surface through `cargo rustics rules`.

## Counting-rule deviations

These deviate from the source's literal definition; the threshold
numbers are unchanged.

### `cyclomatic-complexity` — sealed-aware

McCabe 1976 counts every `match` arm in `d`. rustics excludes arm count
from `match` expressions whose arm set has no wildcard (`_`)
catch-all — Rust enforces exhaustiveness at compile time, so the "did
I forget a case" reading load case-arm count was meant to flag is not
there. `match` *with* a wildcard contributes `arms − 1`. Branches,
loops, `?`, `&&` / `||` each add `+1` as in the original. Code:
`crates/rustics/src/metrics/cyclomatic_complexity.rs`.

### `panic-density` — `unwrap_or`-aware

The literal reading would count every `.unwrap*()` call on `Option` /
`Result`. rustics excludes `.unwrap_or(...)` / `.unwrap_or_else(...)`
/ `.unwrap_or_default()` because they cannot panic by construction —
they are total functions in disguise. Counted: `.unwrap()`,
`.expect(...)`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`,
and `assert*!` / `debug_assert*!` macro family. Code:
`crates/rustics/src/metrics/panic_density.rs`.

### `efferent-coupling` — outer-path only

Martin's Ce counts distinct *external module roots* a file imports.
The naive walker treats every leaf identifier in a `use` group as a
root, so `use foo::{A, B, C}` was counted as 4 dependencies (`foo`,
`A`, `B`, `C`) instead of 1 on `foo`. The fix only recurses into
`use_tree_list` when the outer tree has *no* path (the top-level
grouped form `use {foo, bar};`); when the outer tree has a path, the
children are members of that path and add nothing to the root set.
Code: `crates/rustics/src/metrics/efferent_coupling.rs` (commit
`bd6e3d4`).

### `afferent-coupling` — workspace-only edges

Martin's Ca counts dependents of a module. rustics scopes Ca to
*workspace* dependents — external crate imports (`std`, `serde`, …)
are out of scope because they are not in the change-impact graph the
metric is meant to surface. Resolution is per-file via the longest-
prefix module-key match against `cargo metadata`. Code:
`crates/cargo-rustics/src/cross_file/coupling.rs`.

### `lcom4` — inherent impl only, methods only

Hitz & Montazeri 1995 take connected components over methods that
share a field or call each other. rustics restricts to *inherent*
`impl` blocks (`impl T { … }`) and skips trait `impl`s — trait method
sets are externally constrained, so disjointness of the cohesion
graph reflects the trait shape rather than the type's design.
Associated `const` / `type` items are also skipped (no behaviour to
cluster). Code: `crates/rustics/src/metrics/lcom4.rs`.

## Threshold calibrations

Where rustics' default deviates from the value the cited source
suggests, the deviation is recorded with self-application data
backing the change.

### `halstead-volume` — 1000 → 1500 / 3000

Halstead 1977 commonly cites `1000` as the cut-off in the literature.
Self-application on this Rust workspace shows ordinary functions
cluster at 700–1500 — a function of Rust's verbose punctuation
vocabulary (`::`, `<`, `>`, `&`, lifetimes, generics) inflating both
`N` and `η`. The defaults are `1500` (warning) / `3000` (error) — the
floor sits above the top of the ordinary cluster so that warnings
fire on token-dense outliers, not on the typical Rust function shape.
Source: `doc/manual.md` "halstead-volume".

### `cyclomatic-complexity` — 10 / 20 (matches McCabe)

McCabe's 1976 typical threshold is `10`; rustics ships `10 / 20`.
Self-application clean. No deviation from the literature.

### `cognitive-complexity` — 15 / 25 (matches Campbell)

Campbell's 2018 SonarSource white paper recommends `15`; rustics
ships `15 / 25`.

### `npath-complexity` — 200 / 1000

Nejmeh 1988 recommends `200`. rustics ships `200` (warning) / `1000`
(error). The `error` step is generous — `200`-`1000` is the band
where readers can still navigate by case structure; past `1000` the
exponential blow-up makes black-box exploration infeasible.

### `wmc` / `rfc` — 50 / 100

CK 1994 + follow-up papers (Basili et al. 1996; Subramanyam & Krishnan
2003) converge on `50` as the warning band. rustics ships `50 / 100`.

### `lcom4` — 2 / 4

Hitz & Montazeri 1995: `LCOM4 ≥ 2` means the impl could split.
Marinescu 2002 treats `LCOM4 ≥ 4` as a code smell. rustics ships
`2 / 4`, mirroring both readings.

### `efferent-coupling` (per-file Ce) — 15 / 30

Martin 1994 doesn't pin a numeric Ce threshold. rustics ships
`15 / 30` based on self-application: ordinary leaf modules cluster
at `0–15`; modules above `15` are typically composing several
internal subsystems, which is the "high Ce" Martin describes.

### `afferent-coupling` (cross-file Ca) — 20 / 40

Martin 1994 again doesn't pin a number. rustics ships `20 / 40`
mirroring Ce's structural intuition (`(20 + 20) → instability 0.5`
sits at the main sequence). **Audit pending:** the 5 Layer-1 modules
(`MetricInput`, `MetricMeasurement`, `MetricCalculator`, visitor
helpers, crate root) sit at Ca = 23–35 and are dismissed as Rust
value-type / free-function shapes that escape Martin's class-OO
"Zone of Pain" failure mode. Whether the threshold needs a Rust-
idiom calibration (raise to 40+ for data-carrier modules) or whether
the dismissal channel is the right home for these is open work —
see "Audit gaps".

### `trait-impl-fanout` — 8 / 16

No academic citation backs a specific number. rustics ships `8 / 16`
on self-application observation: traits with ≥ 8 implementors in one
workspace tend to be either canonical seams (intentional, dismiss)
or accidentally bloated. **Audit pending:** community-formal source
not yet cited in code.

## Off-by-default / informational lenses

| Lens | Reason |
| --- | --- |
| `abstractness` | Martin 1994 informational signal; the actionable derived metric (Distance from Main Sequence) was dropped under multicollinearity. Kept as an informational measurement so the report still surfaces `A` for the AI to read. |

## Intentionally absent

| Lens / signal | Reason |
| --- | --- |
| Distance from Main Sequence (`D = \|A + I − 1\|`) — Martin 1994 | Implemented and *removed*. Self-application showed `D ↔ I` correlation `r = −0.994` (n = 86) — Rust's typical Abstractness distribution clusters near 0, so `D` collapses to `1 − I`. Two metrics naming the same thing distorts AI multivariate judgment. Kept `I` (the simpler, more direct "how unstable" reading). The removal is the canonical example of multicollinearity acting on the catalogue. |
| Maximum Nesting Level — *no peer-reviewed primary source* | Implemented and *removed*. Cited "NIST SP 500-235 §4" turned out to be misattribution (§4 of that document is "Simplified Complexity Calculation", not nesting research). No peer-reviewed paper establishes a defect-correlated threshold for raw nesting depth. Self-application also showed `r = 0.74` correlation with `cognitive-complexity`, which already weights nesting into its score — so removing the standalone lens does not lose orthogonal signal. Dropped rather than shipped on convention-only backing. |
| `impl-trait-fanout` / `dyn-density` — *no citation, informational shape probe* | Implemented and *removed*. Counts of `impl Trait` / `dyn Trait` occurrences in signatures. Pure shape probes; no peer-reviewed source ties either count to a defect-correlated threshold, and the values surfaced only through `rustContext` (informational). Removed under the citation rule. If the dispatch-shape signal proves valuable later, reintroduction needs at least an Effective Rust / Rust API Guidelines anchor. |
| `trait-default-impl-ratio` — *no citation, informational* | Implemented and *removed*. Ratio of methods with default bodies vs total trait methods. Informational shape probe; no peer-reviewed source establishes a defect-correlated threshold. Removed under the citation rule. |
| `proc-macro-presence` — *no citation, file-shape probe* | Implemented and *removed*. "Is this function shaped by a proc-macro?" file-shape probe. Informational; useful for AI agents to know but no peer-reviewed source. The same information is recoverable by inspecting attribute lists directly. |
| `borrow-profile-owned` / `-borrowed` / `-mut` — *no citation, three-lens for one informational signal* | Implemented and *removed*. Three lenses (owned / borrowed / mut-borrowed) for what is one informational signal: the ratio across parameter-passing modes. Lacked citation, did not fire violations, and the rustContext surface they fed was redundant with direct AST inspection. Removed; if the signal returns it should be one informational measurement, not three. |
| `match-arm-count` — *r = 0.79 with `cyclomatic-complexity`, no citation* | Implemented and *removed*. The sealed-aware CC lens already absorbs match-arm breadth (it counts only wildcard-bearing matches, identical to this lens's gate). Self-application showed `r = 0.79` between the two — same axis, different name. Without citation backing for the standalone reading, removed under multicollinearity + citation rule. |
| `early-return-density` — *r = 0.77 with `result-chain-depth`, no citation* | Implemented and *removed*. Counted explicit `return` statements; `?` chains are already covered by `result-chain-depth` and CC. No peer-reviewed or community-formal source for the standalone reading. Removed. |
| `impl-length` — *r = 0.86 with `rfc`, r = 0.81 with `wmc`, no citation* | Implemented and *removed*. Informational raw line count of an `impl` block. Heavily correlated with both WMC (CK 1994) and RFC (CK 1994) — those are the citation-backed gates. Removed under multicollinearity + citation rule. |
| Depth of Inheritance Tree (DIT) — CK 1994 | Rust has no inheritance; trait + composition culture keeps any inheritance-shaped reading degenerate. |
| Number of Children (NOC) — CK 1994 | Same reason as DIT. |
| Halstead Difficulty / Effort — Halstead 1977 | Pure derivations of `(η₁, η₂, N₁, N₂)` — no orthogonal signal beyond Halstead Volume. |
| Maintainability Index — Oman & Hagemeister 1992 | Linear combination of `CC + V + LOC` — no orthogonal signal beyond its three components, all of which ship as separate lenses. |
| LCOM1 / LCOM2 — Chidamber & Kemerer 1994 | Hitz & Montazeri 1995 demonstrated that LCOM1/2/3 produce artefacts (zero values for impls that are clearly cohesion-violating); LCOM4 is the corrected formulation. We ship LCOM4 only. |
| Distance and `boolean-trap`-style positional-bool count | No peer-reviewed source establishes a defect-correlated threshold. The `clippy::fn_params_excessive_bools` lint covers the rule-shape side of this signal — separate tool, separate dispatch (rustics measures, clippy lints — see AGENTS.md). |
| `n-path` extended (Bang 1997) | `npath-complexity` (Nejmeh 1988) is the version with established thresholds; the Bang variant adds no thresholded signal we'd act on. |

## Audit gaps

Honest record of what's not yet calibrated to the standard above.

### Citations not recorded in code

The following lenses ship with empty `REFERENCES` constants in their
metric files even though the manual cites a community-formal source
or a convention. This is a documentation defect, not a selection
defect — the lens *is* citation-backed; the citation just isn't in
the binary's exposed `references` field. Fix: copy the manual's
"References." line into the metric module's `REFERENCES` constant
and verify it surfaces through `cargo rustics rules`.

`source-lines-of-code`, `lifetime-arity`, `generic-arity`,
`clone-density`, `unsafe-block-scope`, `panic-density`,
`result-chain-depth`, `await-depth`, `closure-arity`,
`format-density`, `iterator-chain-length`,
`boxed-allocation-density`, `macro-rules-arm-count`,
`trait-method-count`, `trait-impl-fanout` (cross-file).

### Threshold calibrations not documented

`halstead-volume` is the only lens with an explicit Calibration note
in the manual. Every other thresholded lens either matches the
literature unchanged (CC, Cognitive, NPATH, WMC, RFC, LCOM4) or
deviates without recording the self-application observation that
backed the deviation. The deviating lenses are listed above in
"Threshold calibrations" with the rationale; the manual's per-lens
sections should be updated to carry a "Calibration note." line where
applicable, mirroring `halstead-volume`'s prose.

### Frame-mismatch in `afferent-coupling`

Self-application reports 5 dismissals on Layer-1 modules
(`MetricInput`, `MetricMeasurement`, `MetricCalculator`, visitor
helpers, crate root). Each sits in Martin's "Zone of Pain"
(`A ≈ 0`, `Ca > 20`, `D ≈ 1`) by the strict OO-class reading, but
the Rust idioms involved (concrete value-type parameter / return
objects, free-function helper modules, public API re-export hub) are
not the failure mode the metric was calibrated against. The
dismissal channel currently absorbs the gap; whether that's the
right long-term answer or whether a shape-aware threshold (data-
carrier struct exempt, free-function module exempt, etc.) should
ship is open.

The shape-aware proposals would be heuristics tuned to Rust idioms
without direct citation backing — they would be a deviation from the
"each lens cites a published source" principle. The honest position
is that `afferent-coupling` is partially mismatched to Rust's
type-shape vocabulary and the dismissal channel records the gap with
per-instance reasons; whether to escalate to a frame change requires
either a Rust-specific source (none currently established) or a
deliberate exception to the citation rule.
