# Calibration

rustics' lens battery is anchored to published sources. This page is the audit trail for that anchoring: what's selected and why, and where the implementation departs from the source's literal definition.

Threshold *numbers* (e.g. CC warn 10, Halstead warn 1500) follow the cited sources where the literature gives one; calibrated deviations carry a "Calibration note" pinning the change to self-application data. What can also differ is *what is counted* — those deviations are listed below with their justification.

## Selection principles

- **Each lens cites a published source.** Either CS literature (McCabe, Halstead, Chidamber & Kemerer, Hitz & Montazeri, Nejmeh, Martin, Campbell / SonarSource) or community-formal sources (Drysdale's *Effective Rust*, Rust API Guidelines). Lenses without a verifiable source are excluded — see "Intentionally absent" for the drop list.
- **Multicollinearity is checked.** Pairs with `|r| ≥ 0.95` on self-application get dropped. Distance from Main Sequence was removed under this rule when it correlated `r = −0.994` with Instability.
- **One lens, one signal.** Lenses that derive purely from already-shipped lenses (Halstead Difficulty/Effort, Maintainability Index = `CC + V + LOC`) add no orthogonal signal and are absent.
- **Idiom-misaligned lenses are excluded.** DIT and NOC describe inheritance depth/breadth; Rust has no inheritance and the trait + composition culture makes both signals empty. Per-file Martin Abstractness collapses to 0 on the bulk of any Rust workspace because there's no "1 class per file" constraint.
- **Off-by-default / informational when overlap is structural.** `instability` (Martin I) ships informational because the per-file granularity makes the paired `(A, I)` plane collapse in Rust; the value still ranks change-impact. `halstead-volume` and `npath-complexity` ship off-by-default because self-application shows them strongly correlated with already-shipped lenses (see "Off-by-default lenses" below).

## Selected lenses

### Function-level (CS literature)

| Lens | Source |
| --- | --- |
| `cyclomatic-complexity` | McCabe 1976 |
| `cognitive-complexity` | Campbell / SonarSource white paper, 2018 — *industry source, not peer-reviewed* |
| `halstead-volume` *(off)* | Halstead 1977 |
| `npath-complexity` *(off)* | Nejmeh 1988 |

### Function-level (Rust idiom — community-formal)

| Lens | Source |
| --- | --- |
| `panic-density` | Drysdale 2024, *Effective Rust* 2nd ed., Item 18: Don't panic |
| `unsafe-block-scope` | Drysdale 2024, *Effective Rust* 2nd ed., Item 16: Avoid writing unsafe code |
| `lifetime-arity` | Drysdale 2024, *Effective Rust* 2nd ed., Item 14: Understand lifetimes |
| `generic-arity` | Drysdale 2024, *Effective Rust* 2nd ed., Item 12: Understand trade-offs between generics and trait objects |
| `iterator-chain-length` | Drysdale 2024, *Effective Rust* 2nd ed., Item 9: Consider using iterator transforms instead of loops |
| `source-lines-of-code` | Boehm 1981, *Software Engineering Economics* (industry convention; SLOC has no single peer-reviewed threshold paper) |

### Class / impl-block level (CS literature)

| Lens | Source |
| --- | --- |
| `lcom4` | Hitz & Montazeri 1995; Marinescu 2002 |
| `wmc` (Weighted Methods per Class) | Chidamber & Kemerer 1994 |
| `rfc` (Response For a Class) | Chidamber & Kemerer 1994 |

### Cross-file / module level (Martin 1994)

| Lens | Notes |
| --- | --- |
| `efferent-coupling` (per-file Ce) | Distinct external module roots a file imports. |
| `afferent-coupling` (cross-file Ca) | Workspace files that depend on this module. |
| `instability` (`I = Ce / (Ca + Ce)`, informational) | Surfaces drift in change-impact ranking. |

Default thresholds and per-lens descriptions live in [`doc/manual.md`](manual.md). Full bibliographic citations are exposed by each lens's `references` getter and surface through `cargo rustics rules`.

## Counting-rule deviations

These deviate from the source's literal definition; threshold numbers are unchanged.

### `cyclomatic-complexity` — sealed-aware

McCabe 1976 counts every `match` arm in `d`. rustics excludes arm count from `match` expressions whose arm set has no wildcard (`_`) catch-all — Rust enforces exhaustiveness at compile time, so the "did I forget a case" reading load case-arm count was meant to flag is not there. `match` *with* a wildcard contributes `arms − 1`. Branches, loops, `?`, `&&` / `||` each add `+1` as in the original. Code: `crates/rustics/src/metrics/cyclomatic_complexity.rs`.

### `panic-density` — `unwrap_or`-aware

The literal reading would count every `.unwrap*()` call on `Option` / `Result`. rustics excludes `.unwrap_or(...)` / `.unwrap_or_else(...)` / `.unwrap_or_default()` because they cannot panic by construction. Counted: `.unwrap()`, `.expect(...)`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`, and `assert*!` / `debug_assert*!` macro family. Code: `crates/rustics/src/metrics/panic_density.rs`.

### `efferent-coupling` — outer-path only

Martin's Ce counts distinct *external module roots* a file imports. The naive walker treats every leaf identifier in a `use` group as a root, so `use foo::{A, B, C}` was counted as 4 dependencies (`foo`, `A`, `B`, `C`) instead of 1 on `foo`. The fix only recurses into `use_tree_list` when the outer tree has *no* path (the top-level grouped form `use {foo, bar};`); when the outer tree has a path, the children are members of that path and add nothing to the root set.

### `afferent-coupling` — workspace-only edges

Martin's Ca counts dependents of a module. rustics scopes Ca to *workspace* dependents — external crate imports (`std`, `serde`, …) are out of scope because they are not in the change-impact graph the metric is meant to surface. Resolution is per-file via the longest-prefix module-key match against `cargo metadata`.

### `lcom4` — inherent impl only

Hitz & Montazeri 1995 take connected components over methods that share a field or call each other. rustics restricts to *inherent* `impl` blocks (`impl T { … }`) and skips trait `impl`s — trait method sets are externally constrained, so disjointness of the cohesion graph reflects the trait shape rather than the type's design.

## Off-by-default lenses

Both ship with `default_warning: None` / `default_error: None`. The lens still runs and emits measurements (so `cargo rustics regression`'s drift detection and `--reporter ai`'s informational surface keep working), but it does not fire warnings unless the user opts in via `[rustics.metrics.<id>]` in `rustics.toml`.

| Lens | Reason for off-by-default |
| --- | --- |
| `halstead-volume` | Self-application shows `r = 0.84` with `source-lines-of-code` (n = 464) — the same "function size" axis measured in a different vocabulary. Alfadel et al. 2017 (*An Empirical Analysis of Halstead's Metrics in Object-Oriented Software*) documents the CC + SLOC + Halstead Volume mutual correlation. Shipping all three would duplicate signal. Recommended threshold for opt-in: `1500 / 3000` (calibrated below). |
| `npath-complexity` | Self-application shows `r = 0.78` with `cognitive-complexity` and `r = 0.61` with `cyclomatic-complexity` (n = 1072 each). Cognitive Complexity (Campbell 2018) is the modern refinement of NPATH-style path counting — it weights nesting and sequencing the way NPATH's flat multiplicative count cannot. NPATH still adds value in the long tail (very large state machines push CC and Cognitive past their ceilings before NPATH); kept opt-in for that case. Recommended threshold for opt-in: `200 / 1000` (Nejmeh's 1988 number). |

## Calibration notes

Where the rustics default deviates from the cited source's number, the deviation is recorded with self-application data backing the change.

### `halstead-volume` — 1000 → 1500 / 3000 (when opted in)

Halstead 1977 commonly cites `1000` as the cut-off in the literature. Self-application on this Rust workspace shows ordinary functions cluster at 700–1500 — a function of Rust's verbose punctuation vocabulary (`::`, `<`, `>`, `&`, lifetimes, generics) inflating both `N` and `η`. The recommended opt-in defaults are `1500` (warning) / `3000` (error) — the floor sits above the top of the ordinary cluster so warnings fire on token-dense outliers, not on the typical Rust function shape. Apply by setting `[rustics.metrics.halstead-volume] warning = 1500` (and optionally `error = 3000`) in `rustics.toml`.

Every other thresholded lens matches the literature unchanged (CC 10/20, Cognitive 15/25, WMC 50/100, RFC 50/100, LCOM4 2/4) or rides on convention-based defaults (Boehm SLOC 60/120; Drysdale-anchored Rust idiom lenses are recent enough that no "ordinary function cluster" study exists in the literature — defaults are calibrated against this codebase's self-application).

## Per-file Martin granularity

Martin's 1994 framework was developed for OO languages where "package = release unit" and a file typically holds one main type (Java's `public class Foo` ↔ `Foo.java` is compiler-enforced). Rust does **not** have that constraint:

- A `.rs` file declares one module, but the module can contain any number of `pub struct` / `pub enum` / `pub trait` / `pub fn` / sub-module items. The idiomatic pattern (concrete struct + impl blocks + helpers in one file) is invisible to Java-style "1 file = 1 type."
- The release unit in Rust is the *crate*, not the file.

dartrics flagged the same mismatch for Dart and turned `abstractness` and `distance-from-main-sequence` off. rustics does the same — `D` was already removed under multicollinearity, and `abstractness` was removed for per-file collapse to 0 on the bulk of the workspace.

`efferent-coupling` (per-file Ce) and `afferent-coupling` (cross-file Ca) and `instability` are kept because the count itself is a useful change-impact ranking even when divorced from Martin's `A`-paired Pain/Uselessness verdicts. They are *not* treated as Martin-frame "is this design good" gates; they are "if you change this file, who breaks?" rankings.

## Role split with Clippy

`rustics measures, clippy lints` (AGENTS.md). Rustics is a quantitative tool — every lens emits a number that crosses a threshold; Clippy is a rule tool — every lint fires when a pattern matches. The two have orthogonal data shapes, orthogonal stable-id semantics, and orthogonal fix profiles.

The closest neighbouring signals between the two:

| Rustics lens | Adjacent Clippy lint(s) | How they compose |
| --- | --- | --- |
| `panic-density` | `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic` | Clippy fires per-occurrence; rustics counts and thresholds at the function. A user who has Clippy's per-occurrence lint allowed will still see the rustics aggregate — they're independent gates. |
| `unsafe-block-scope` | `clippy::undocumented_unsafe_blocks` | Clippy is a rule on the documentation pattern; rustics is a count of unsafe lines. The two share no shape. |
| `lifetime-arity` | `clippy::needless_lifetimes` | Clippy removes elidable lifetimes; the lens counts what remains. Compose: run Clippy first, then read the lens's count of *genuine* explicit lifetimes. |
| `iterator-chain-length` | `clippy::needless_collect`, `clippy::iter_*` family | Clippy fires on specific anti-patterns; the lens counts overall chain length. Different signals. |

Most rustics function-level lenses (CC, NPATH, SLOC, Cognitive, Halstead, generic-arity) have no direct Clippy counterpart — Clippy doesn't measure "how complex is this function as a whole," it fires on specific code smells. The two tools are complementary, not redundant.

## Intentionally absent

| Lens / signal | Reason |
| --- | --- |
| Distance from Main Sequence (`D = \|A + I − 1\|`) — Martin 1994 | Implemented and *removed*. Self-application showed `D ↔ I` correlation `r = −0.994`; Rust's typical Abstractness distribution clusters near 0, so `D` collapses to `1 − I`. The canonical example of multicollinearity acting on the catalogue. |
| `abstractness` (Martin A) — Martin 1994 | Per-file granularity collapses to 0 on the bulk of any Rust workspace; see "Per-file Martin granularity" above. |
| `maximum-nesting-level` | The widely cited "NIST SP 500-235 §4" attribution is incorrect (§4 of that document is "Simplified Complexity Calculation", not nesting research); no peer-reviewed paper establishes a defect-correlated threshold for raw nesting depth. Self-application also showed `r = 0.74` with `cognitive-complexity`, which already weights nesting. |
| Halstead Difficulty / Effort | Pure derivations of `(η₁, η₂, N₁, N₂)` — no orthogonal signal beyond Halstead Volume. |
| Maintainability Index — Oman & Hagemeister 1992 | Linear combination of `CC + V + LOC` — no orthogonal signal beyond its three components, all of which ship as separate lenses. |
| LCOM1 / LCOM2 / LCOM3 — CK 1994; Li & Henry 1993 | Hitz & Montazeri 1995 demonstrated systematic artefacts in earlier LCOM variants; LCOM4 is the corrected formulation and is the only one shipped. |
| DIT (Depth of Inheritance Tree) / NOC (Number of Children) — CK 1994 | Rust has no inheritance; trait + composition culture keeps any inheritance-shaped reading degenerate. |
| `n-path` extended (Bang 1997) | `npath-complexity` (Nejmeh 1988) is the version with established thresholds. |
| `boolean-trap`-style positional-bool count | No peer-reviewed source establishes a defect-correlated threshold; `clippy::fn_params_excessive_bools` covers the rule-shape side of this signal. Use that lint instead. |
