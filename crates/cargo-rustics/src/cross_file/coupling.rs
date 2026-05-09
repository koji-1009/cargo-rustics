//! Cross-file Martin coupling metrics: Afferent Coupling (Ca).
//!
//! Plan §6.3. The per-file `efferent-coupling` lens counts each
//! `use <root>::…` once per leftmost root and runs file-locally.
//! The *afferent* counterpart asks the inverse question — "how many
//! other files in this workspace depend on me?" — and therefore must
//! see the whole file set.
//!
//! ## Granularity
//!
//! Granularity is per-file. Each `.rs` file is treated as a module
//! identified by `<crate-name>::<module-path>` (the same prefix the
//! `analyze` command uses to anchor measurements). External crates
//! (`std`, `serde`, …) do not have a Ca because they are outside the
//! workspace.
//!
//! ## Resolution algorithm
//!
//! 1. Read the workspace's crate names from `cargo_metadata`.
//! 2. For each file F, derive its module identity from its relative
//!    path. Build the index `(crate, module-path) → idx`.
//! 3. For each file F's `use` statements, normalise the target root
//!    to a workspace crate name (`crate::` → F's own crate; bare
//!    workspace-name → that crate; everything else → external,
//!    ignored).
//! 4. Walk the longest-prefix match in the index: `crate::a::b::c` →
//!    file at `(crate, a::b::c)`, else `(crate, a::b)`, else
//!    `(crate, a)`, else `(crate, "")` (lib root). The first hit is
//!    the depended-on module.
//! 5. Ca(M) = number of *other* files whose dependency set contains
//!    M's module identity.
//!
//! References:
//! * Martin, R. C. (1994). OO Design Quality Metrics: An Analysis of
//!   Dependencies.
//! * plan §6.3 — Afferent Coupling (Ca).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use cargo_metadata::MetadataCommand;
use rustics::{violation_id, ScopeKind};
use syn::{Item, UseTree};

use crate::discover::DiscoveredFile;
use crate::report::{MeasurementRecord, Violation};

use super::CrossFilePass;

/// Same-eye thresholds, mirroring the per-file Ce defaults.
const AFFERENT_COUPLING_WARNING: u32 = 20;
const AFFERENT_COUPLING_ERROR: u32 = 40;

// Distance-from-Main-Sequence (D = |A + I − 1|) was implemented and
// then *removed* under the multicollinearity rule. Self-application
// showed `D ↔ instability r = −0.994` (n = 86). Mathematically:
// when A ≈ 0 (the natural Rust pattern of struct-only data-carrier
// modules), D collapses to `1 − I`. Two metrics that say the same
// thing distort multivariate AI judgment, so the redundant one
// goes — keeping I (the simpler, more direct "how unstable is this
// module" signal). If the underlying A-distribution shifts in a
// future codebase such that D and I decorrelate, D can come back.

/// Walks every discovered file, resolves `use`-graph edges within
/// the workspace, and emits per-module Ca + Instability output.
/// Each module gets:
/// * One `afferent-coupling` measurement (always — so `regression`
///   sees sub-threshold Ca drifts).
/// * One `afferent-coupling` violation if Ca > warning.
/// * One `instability` measurement (informational only).
///
/// On cargo-metadata failure (running outside a Cargo workspace)
/// the pass degrades gracefully — without a crate map we cannot
/// resolve workspace edges, so the function returns an empty
/// result.
pub fn run(workspace_root: &Path, files: &[DiscoveredFile]) -> CrossFilePass {
    let crate_names = read_crate_names(workspace_root).unwrap_or_default();
    let modules = build_module_index(files, &crate_names);
    let module_keys: HashSet<(String, String)> =
        modules.iter().map(ModuleEntry::key).collect();
    let key_to_idx: BTreeMap<(String, String), usize> = modules
        .iter()
        .enumerate()
        .map(|(i, m)| (m.key(), i))
        .collect();
    let dependencies = build_dependency_graph(&modules, &module_keys, &crate_names);
    let ca = count_afferent(&dependencies, &key_to_idx, modules.len());
    let ce_internal = count_efferent_internal(&dependencies, modules.len());
    let instability = compute_instability(&ce_internal, &ca);
    let violations = emit_violations(&modules, &ca);
    let mut measurements = emit_ca_measurements(&modules, &ca);
    measurements.extend(emit_instability_measurements(&modules, &instability));
    CrossFilePass {
        violations,
        measurements,
    }
}

/// Workspace crate names from cargo metadata.
fn read_crate_names(workspace_root: &Path) -> Result<HashSet<String>> {
    let manifest = workspace_root.join("Cargo.toml");
    let metadata = MetadataCommand::new()
        .manifest_path(manifest)
        .no_deps()
        .exec()?;
    Ok(metadata
        .workspace_packages()
        .into_iter()
        .map(|p| p.name.to_string())
        .collect())
}

/// Per-file module entry — the unit of Ca measurement.
#[derive(Debug, Clone)]
struct ModuleEntry {
    /// Workspace-relative file path (the report's anchor).
    relative: String,
    /// Absolute path on disk (used to read the file's `use`s).
    absolute: PathBuf,
    /// Workspace crate this file belongs to (e.g. `cargo-rustics`).
    crate_name: String,
    /// `::`-joined module path *within* the crate; empty for
    /// lib/main roots. Examples: "" (crate root), "metrics",
    /// "metrics::lcom4".
    module_path: String,
}

impl ModuleEntry {
    /// Identity used as both the dependency-graph key and the
    /// `(crate, module_path)` index lookup.
    fn key(&self) -> (String, String) {
        (self.crate_name.clone(), self.module_path.clone())
    }
}

fn build_module_index(
    files: &[DiscoveredFile],
    crate_names: &HashSet<String>,
) -> Vec<ModuleEntry> {
    files
        .iter()
        .map(|file| {
            let (crate_name, module_path) = derive_module_identity(&file.relative, crate_names);
            ModuleEntry {
                relative: file.relative.clone(),
                absolute: file.absolute.clone(),
                crate_name,
                module_path,
            }
        })
        .collect()
}

/// Derives `(crate_name, module_path)` from a workspace-relative
/// file path. For paths outside any `src/` directory the parent
/// dir name is used as a synthetic crate so the file still appears
/// in the index (it can never be a target — there's no `use` syntax
/// to reach it — but it can still emit outgoing edges).
fn derive_module_identity(
    relative: &str,
    crate_names: &HashSet<String>,
) -> (String, String) {
    let parts: Vec<&str> = relative.split('/').collect();
    let Some(src_idx) = parts.iter().position(|p| *p == "src") else {
        let synthetic = parts.get(parts.len().saturating_sub(2)).copied().unwrap_or("");
        return (synthetic.to_string(), String::new());
    };
    let crate_dir = if src_idx > 0 { parts[src_idx - 1] } else { "" };
    let crate_name = if crate_names.contains(crate_dir) {
        crate_dir.to_string()
    } else {
        // Fall back to one level up — workspaces sometimes nest under
        // a category dir (`crates/<NAME>/src/...`).
        let parent = if src_idx >= 2 { parts[src_idx - 2] } else { "" };
        if crate_names.contains(parent) {
            parent.to_string()
        } else {
            crate_dir.to_string()
        }
    };
    let module_path = module_path_after_src(&parts[src_idx + 1..]);
    (crate_name, module_path)
}

/// Translates the path components after `src/` into a Rust module
/// path. `lib.rs` / `main.rs` / `mod.rs` collapse to the empty
/// string (the crate root or the parent directory's module).
fn module_path_after_src(after_src: &[&str]) -> String {
    let mut segments: Vec<String> = after_src.iter().map(|s| s.to_string()).collect();
    if let Some(last) = segments.last_mut() {
        if let Some(stripped) = last.strip_suffix(".rs") {
            *last = stripped.to_string();
        }
    }
    if matches!(
        segments.last().map(String::as_str),
        Some("lib" | "main" | "mod")
    ) {
        segments.pop();
    }
    segments.join("::")
}

/// Per-file outgoing dependencies. Keyed by file index.
type DepGraph = BTreeMap<usize, BTreeSet<(String, String)>>;

fn build_dependency_graph(
    modules: &[ModuleEntry],
    module_keys: &HashSet<(String, String)>,
    crate_names: &HashSet<String>,
) -> DepGraph {
    let mut deps: DepGraph = BTreeMap::new();
    for (i, entry) in modules.iter().enumerate() {
        let Some(targets) = read_use_targets(entry, module_keys, crate_names) else {
            continue;
        };
        // Self-edges don't make sense — a file does not depend on itself.
        let targets: BTreeSet<_> =
            targets.into_iter().filter(|t| t != &entry.key()).collect();
        if !targets.is_empty() {
            deps.insert(i, targets);
        }
    }
    deps
}

/// Reads `module`'s file from disk, parses it, and resolves every
/// `use` statement to a workspace module key.
fn read_use_targets(
    module: &ModuleEntry,
    module_keys: &HashSet<(String, String)>,
    crate_names: &HashSet<String>,
) -> Option<BTreeSet<(String, String)>> {
    let source = std::fs::read_to_string(&module.absolute).ok()?;
    let ast = syn::parse_file(&source).ok()?;
    let mut out = BTreeSet::new();
    for item in &ast.items {
        if let Item::Use(u) = item {
            collect_use_targets(
                &u.tree,
                Vec::new(),
                module,
                module_keys,
                crate_names,
                &mut out,
            );
        }
    }
    Some(out)
}

fn collect_use_targets(
    tree: &UseTree,
    prefix: Vec<String>,
    module: &ModuleEntry,
    module_keys: &HashSet<(String, String)>,
    crate_names: &HashSet<String>,
    out: &mut BTreeSet<(String, String)>,
) {
    match tree {
        UseTree::Path(p) => {
            let mut next = prefix;
            next.push(p.ident.to_string());
            collect_use_targets(&p.tree, next, module, module_keys, crate_names, out);
        }
        UseTree::Name(n) => {
            let mut full = prefix;
            full.push(n.ident.to_string());
            resolve_full_path(&full, module, module_keys, crate_names, out);
        }
        UseTree::Rename(r) => {
            let mut full = prefix;
            full.push(r.ident.to_string());
            resolve_full_path(&full, module, module_keys, crate_names, out);
        }
        UseTree::Glob(_) => {
            resolve_full_path(&prefix, module, module_keys, crate_names, out);
        }
        UseTree::Group(g) => {
            for item in &g.items {
                collect_use_targets(item, prefix.clone(), module, module_keys, crate_names, out);
            }
        }
    }
}

/// Outcome of normalising a `use`'s leading segment.
enum PathTarget<'a> {
    /// The path resolves into a workspace crate. The first field is
    /// the target crate name; the second is the remainder of the
    /// path (excluding the leading segment when consumed). The
    /// boolean is `true` when we are *certain* the path is intra-
    /// workspace — only then is the crate-root fallback safe.
    Internal {
        target_crate: String,
        rest: &'a [String],
        certain: bool,
    },
    /// `super::…` / explicitly external — emit nothing.
    Skip,
}

/// Converts a `use` path into a `(crate, module-path)` key when the
/// target lives in this workspace. External paths return silently.
fn resolve_full_path(
    segments: &[String],
    module: &ModuleEntry,
    module_keys: &HashSet<(String, String)>,
    crate_names: &HashSet<String>,
    out: &mut BTreeSet<(String, String)>,
) {
    if segments.is_empty() {
        return;
    }
    let target = classify_use_root(segments, module, crate_names);
    let PathTarget::Internal { target_crate, rest, certain } = target else {
        return;
    };
    if let Some(key) = longest_prefix_match(&target_crate, rest, module_keys) {
        out.insert(key);
        return;
    }
    // Crate-root fallback only when the leading segment proves the
    // path is intra-workspace. Otherwise an external `use std::X`
    // would land on the current crate's lib.rs.
    if certain {
        let root_key = (target_crate, String::new());
        if module_keys.contains(&root_key) {
            out.insert(root_key);
        }
    }
}

/// Inspects the leading segment of `segments` and decides which
/// crate / remainder / certainty the resolver should walk.
fn classify_use_root<'a>(
    segments: &'a [String],
    module: &ModuleEntry,
    crate_names: &HashSet<String>,
) -> PathTarget<'a> {
    match segments[0].as_str() {
        "crate" | "self" => PathTarget::Internal {
            target_crate: module.crate_name.clone(),
            rest: &segments[1..],
            certain: true,
        },
        "super" => PathTarget::Skip,
        s if crate_names.contains(s) => PathTarget::Internal {
            target_crate: s.to_string(),
            rest: &segments[1..],
            certain: true,
        },
        // Rust 2018 relative path (`use metrics::X` from inside
        // the same crate) *or* an external (std / serde / anyhow).
        // We probe the current crate, but only commit to an edge
        // if the longest-prefix walk hits a real submodule — never
        // the crate root, since that would route every external
        // `use` to lib.rs.
        _ => PathTarget::Internal {
            target_crate: module.crate_name.clone(),
            rest: segments,
            certain: false,
        },
    }
}

/// Walks the path tail right-to-left and returns the first
/// `(target_crate, prefix)` key that exists in `module_keys`.
fn longest_prefix_match(
    target_crate: &str,
    rest: &[String],
    module_keys: &HashSet<(String, String)>,
) -> Option<(String, String)> {
    let mut path: Vec<String> = rest.to_vec();
    while !path.is_empty() {
        let key = (target_crate.to_string(), path.join("::"));
        if module_keys.contains(&key) {
            return Some(key);
        }
        path.pop();
    }
    None
}

fn count_afferent(
    deps: &DepGraph,
    key_to_idx: &BTreeMap<(String, String), usize>,
    module_count: usize,
) -> Vec<u32> {
    let mut ca = vec![0u32; module_count];
    for targets in deps.values() {
        for key in targets {
            if let Some(&j) = key_to_idx.get(key) {
                ca[j] = ca[j].saturating_add(1);
            }
        }
    }
    ca
}

/// Workspace-internal Ce per module — the cardinality of each
/// module's outgoing dependency set. Diverges from the per-file
/// `efferent-coupling` lens, which counts every leftmost root
/// (including external crates like `std` and `serde`); Martin's
/// Instability ratio is defined over *same-system* dependencies so
/// we use the workspace-internal subset.
fn count_efferent_internal(deps: &DepGraph, module_count: usize) -> Vec<u32> {
    let mut ce = vec![0u32; module_count];
    for (&i, targets) in deps {
        ce[i] = targets.len() as u32;
    }
    ce
}

/// Per-module Instability `I = Ce / (Ce + Ca)`. Martin 1994:
/// 0 → totally stable (depended on, doesn't depend out);
/// 1 → totally unstable (depends out, no incoming dependents).
/// Modules with `Ce = Ca = 0` are isolated; we report `I = 0` for
/// them by convention (the value is informational and the AI report
/// reads it alongside Ca / Ce / A for the full picture).
fn compute_instability(ce_internal: &[u32], ca: &[u32]) -> Vec<f64> {
    ce_internal
        .iter()
        .zip(ca.iter())
        .map(|(&ce, &ca)| {
            let total = ce + ca;
            if total == 0 {
                0.0
            } else {
                f64::from(ce) / f64::from(total)
            }
        })
        .collect()
}

fn emit_instability_measurements(
    modules: &[ModuleEntry],
    instability: &[f64],
) -> Vec<MeasurementRecord> {
    modules
        .iter()
        .zip(instability.iter())
        .map(|(m, &i)| MeasurementRecord {
            file: m.relative.clone(),
            scope: m.module_path.clone(),
            metric: "instability".into(),
            value: i,
        })
        .collect()
}

/// Per-module Ca measurements — one entry per file, regardless of
/// whether the count crosses the warning threshold. The pre-merge
/// version of this lens emitted only violations, leaving
/// `regression`'s cosmetic-detection blind to sub-threshold drifts
/// (`Ca: 12 → 13` invisible). Now every module appears.
fn emit_ca_measurements(
    modules: &[ModuleEntry],
    ca: &[u32],
) -> Vec<MeasurementRecord> {
    modules
        .iter()
        .zip(ca.iter())
        .map(|(m, &count)| MeasurementRecord {
            file: m.relative.clone(),
            scope: m.module_path.clone(),
            metric: "afferent-coupling".into(),
            value: f64::from(count),
        })
        .collect()
}

fn emit_violations(modules: &[ModuleEntry], ca: &[u32]) -> Vec<Violation> {
    let mut out = Vec::new();
    for (i, entry) in modules.iter().enumerate() {
        let count = ca[i];
        let Some((severity, threshold)) = super::severity_for(
            count,
            AFFERENT_COUPLING_WARNING,
            AFFERENT_COUPLING_ERROR,
        ) else {
            continue;
        };
        // Match the per-file lens convention (efferent-coupling /
        // abstractness emit the within-crate module path without
        // the crate prefix; the file path disambiguates cross-crate
        // collisions). Fully-qualified path stays in the rationale.
        let scope = entry.module_path.clone();
        let id = violation_id(&entry.relative, &scope, "afferent-coupling");
        out.push(Violation {
            id,
            file: entry.relative.clone(),
            line: 1,
            scope,
            scope_kind: ScopeKind::Module,
            metric: "afferent-coupling".into(),
            value: f64::from(count),
            threshold: f64::from(threshold),
            severity,
            rationale: Some(rationale_for(entry, count)),
            refactor_hints: REFACTOR_HINTS.iter().map(|s| s.to_string()).collect(),
            references: REFERENCES.iter().map(|s| s.to_string()).collect(),
            rust_context: Default::default(),
            complexity_justified: None,
        });
    }
    out
}

fn rationale_for(entry: &ModuleEntry, count: u32) -> String {
    let path = if entry.module_path.is_empty() {
        format!("{} (crate root)", entry.crate_name)
    } else {
        format!("{}::{}", entry.crate_name, entry.module_path)
    };
    format!(
        "{count} workspace files import from `{path}`. A high \
afferent-coupling means many places in the codebase break if you \
change this module's public surface — invest in narrow APIs, \
backwards-compatible changes, or splitting the module."
    )
}

const REFACTOR_HINTS: &[&str] = &[
    "If many files reach into a single deep symbol of this module, \
publish a focused re-export at a stable path so the spread of \
transitive dependents narrows.",
    "Modules with high Ca pair well with high abstractness (A): \
keep the module's public surface trait-shaped so dependents bind \
to a contract, not a concrete implementation.",
    "If the module has both high Ca and high Ce (= high coupling \
in both directions), it is a likely 'central hub' — consider \
splitting it by role.",
];

const REFERENCES: &[&str] = &[
    "Martin, R. C. (1994). OO Design Quality Metrics: An Analysis of Dependencies.",
    "plan §6.3 — Afferent Coupling (Ca).",
];

#[cfg(test)]
mod tests {
    use super::*;
    use rustics::MetricSeverity;

    #[test]
    fn module_path_collapses_lib_main_mod() {
        assert_eq!(module_path_after_src(&["lib.rs"]), "");
        assert_eq!(module_path_after_src(&["main.rs"]), "");
        assert_eq!(module_path_after_src(&["a", "mod.rs"]), "a");
        assert_eq!(module_path_after_src(&["a", "b.rs"]), "a::b");
    }

    #[test]
    fn derive_identity_for_known_crate() {
        let mut names = HashSet::new();
        names.insert("rustics".to_string());
        let (c, m) = derive_module_identity("crates/rustics/src/metrics/lcom4.rs", &names);
        assert_eq!(c, "rustics");
        assert_eq!(m, "metrics::lcom4");
    }

    #[test]
    fn derive_identity_falls_back_to_dir_name_when_unknown() {
        let names = HashSet::new();
        let (c, m) = derive_module_identity("foo/src/lib.rs", &names);
        assert_eq!(c, "foo");
        assert_eq!(m, "");
    }

    // (severity-ladder tests live with the shared `super::severity_for`
    // in `cross_file/mod.rs`.)

    fn keys_of(pairs: &[(&str, &str)]) -> HashSet<(String, String)> {
        pairs
            .iter()
            .map(|(c, p)| ((*c).to_string(), (*p).to_string()))
            .collect()
    }

    fn names_of(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    fn module_for(crate_name: &str, module_path: &str) -> ModuleEntry {
        ModuleEntry {
            relative: format!("crates/{crate_name}/src/{module_path}.rs"),
            absolute: PathBuf::new(),
            crate_name: crate_name.into(),
            module_path: module_path.into(),
        }
    }

    fn segs(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn external_use_does_not_fall_back_to_crate_root() {
        // Pre-fix this resolved `std::collections::HashMap` from
        // inside crate `rustics` to `(rustics, "")` (= lib.rs),
        // inflating Ca on every crate root by every external use.
        // Now: external paths are silently dropped.
        let module_keys = keys_of(&[("rustics", ""), ("rustics", "metrics")]);
        let crate_names = names_of(&["rustics"]);
        let module = module_for("rustics", "foo");
        let mut out = BTreeSet::new();
        resolve_full_path(
            &segs(&["std", "collections", "HashMap"]),
            &module,
            &module_keys,
            &crate_names,
            &mut out,
        );
        assert!(
            out.is_empty(),
            "external `std::*` must not resolve to the crate root: {out:?}"
        );
    }

    #[test]
    fn intra_crate_relative_path_resolves_to_internal_module() {
        // `use metrics::X` from inside the rustics crate (Rust 2018)
        // should resolve to `(rustics, "metrics")` — not external.
        let module_keys = keys_of(&[("rustics", ""), ("rustics", "metrics")]);
        let crate_names = names_of(&["rustics"]);
        let module = module_for("rustics", "lib");
        let mut out = BTreeSet::new();
        resolve_full_path(
            &segs(&["metrics", "Foo"]),
            &module,
            &module_keys,
            &crate_names,
            &mut out,
        );
        assert!(
            out.contains(&("rustics".into(), "metrics".into())),
            "intra-crate relative path missed: {out:?}"
        );
    }

    #[test]
    fn workspace_crate_use_resolves_to_root_when_no_submodule_match() {
        // `use rustics::CyclomaticComplexity` from another crate,
        // where the symbol is re-exported at the rustics crate root,
        // must resolve to `(rustics, "")`.
        let module_keys = keys_of(&[("rustics", ""), ("rustics", "metrics")]);
        let crate_names = names_of(&["rustics", "cargo-rustics"]);
        let module = module_for("cargo-rustics", "main");
        let mut out = BTreeSet::new();
        resolve_full_path(
            &segs(&["rustics", "CyclomaticComplexity"]),
            &module,
            &module_keys,
            &crate_names,
            &mut out,
        );
        assert!(
            out.contains(&("rustics".into(), "".into())),
            "crate-root fallback for workspace crate missed: {out:?}"
        );
    }

    #[test]
    fn super_path_is_silently_dropped() {
        // Documented limitation: `super::x` is not resolved
        // (the parent context isn't currently threaded through).
        let mut out = BTreeSet::new();
        let module_keys = keys_of(&[("foo", "bar")]);
        let crate_names = names_of(&["foo"]);
        let module = module_for("foo", "bar::child");
        resolve_full_path(
            &segs(&["super", "Item"]),
            &module,
            &module_keys,
            &crate_names,
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn instability_endpoints_match_definition() {
        // Totally stable: Ce=0, Ca=5 → I = 0.
        assert_eq!(compute_instability(&[0], &[5]), vec![0.0]);
        // Totally unstable: Ce=5, Ca=0 → I = 1.
        assert_eq!(compute_instability(&[5], &[0]), vec![1.0]);
        // Balanced: Ce=Ca=3 → I = 0.5.
        assert_eq!(compute_instability(&[3], &[3]), vec![0.5]);
        // Isolated: 0/0 fallback → 0.
        assert_eq!(compute_instability(&[0], &[0]), vec![0.0]);
    }

    #[test]
    fn instability_emits_one_measurement_per_module() {
        let modules = vec![
            ModuleEntry {
                relative: "src/a.rs".into(),
                absolute: PathBuf::new(),
                crate_name: "x".into(),
                module_path: "a".into(),
            },
            ModuleEntry {
                relative: "src/b.rs".into(),
                absolute: PathBuf::new(),
                crate_name: "x".into(),
                module_path: "b".into(),
            },
        ];
        let i = vec![0.25, 0.75];
        let out = emit_instability_measurements(&modules, &i);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].metric, "instability");
        assert_eq!(out[0].scope, "a");
        assert_eq!(out[0].value, 0.25);
        assert_eq!(out[1].value, 0.75);
    }

    /// End-to-end smoke test: build a tiny synthetic workspace,
    /// where 25 files all `use crate::core` so the `core` module
    /// crosses the warning threshold.
    #[test]
    fn afferent_coupling_aggregates_across_files() {
        // Construct module entries directly — bypasses cargo-metadata
        // and disk IO so the test is hermetic.
        let mut modules: Vec<ModuleEntry> = Vec::new();
        let core = ModuleEntry {
            relative: "src/core.rs".into(),
            absolute: PathBuf::from("/tmp/non-existent-core"),
            crate_name: "x".into(),
            module_path: "core".into(),
        };
        modules.push(core);
        for i in 0..25 {
            modules.push(ModuleEntry {
                relative: format!("src/dep_{i}.rs"),
                absolute: PathBuf::from(format!("/tmp/non-existent-dep-{i}")),
                crate_name: "x".into(),
                module_path: format!("dep_{i}"),
            });
        }
        // Hand-build a dependency graph: every dep points to `core`.
        let mut deps: DepGraph = BTreeMap::new();
        let core_key = ("x".into(), "core".into());
        for i in 1..=25 {
            let mut s = BTreeSet::new();
            s.insert(core_key.clone());
            deps.insert(i, s);
        }
        let key_to_idx: BTreeMap<_, _> =
            modules.iter().enumerate().map(|(i, m)| (m.key(), i)).collect();
        let ca = count_afferent(&deps, &key_to_idx, modules.len());
        assert_eq!(ca[0], 25, "core should be depended on by all 25 dep files");
        let violations = emit_violations(&modules, &ca);
        let v = violations
            .iter()
            .find(|v| v.scope == "core")
            .expect("warning violation");
        assert_eq!(v.severity, MetricSeverity::Warning);
        assert_eq!(v.value, 25.0);
        assert_eq!(v.threshold, f64::from(AFFERENT_COUPLING_WARNING));
        assert_eq!(v.metric, "afferent-coupling");
    }
}
