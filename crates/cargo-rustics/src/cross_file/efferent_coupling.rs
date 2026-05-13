//! HIR-aware Efferent Coupling (Ce) — per-file count of distinct
//! external crates the file's `use` statements reach.
//!
//! Replaces the per-file AST walker in `rustics::metrics::
//! efferent_coupling`. Two accuracy gains documented in
//! `tmp/ra-ap-spike-notes.md`:
//!
//! * `use crate::foo::Bar` resolves to this crate, so it doesn't
//!   inflate Ce. The AST walker counts `crate` as a root.
//! * `use facade::ReExported` resolves to the *real* origin crate,
//!   not the re-export hop. The AST walker counts the leftmost
//!   segment regardless of where the imported item actually lives.
//!
//! Thresholds match the AST lens (`warning = 15`, `error = 30`).

use std::path::Path;

use rustics::{violation_id, MetricSeverity, ScopeKind};

use crate::discover::DiscoveredFile;
use crate::report::{MeasurementRecord, Violation};

use super::CrossFilePass;

/// Same thresholds as the per-file AST walker — the HIR version
/// just measures more accurately. Keeping the numbers stable means
/// users who upgrade do not see threshold-crossings appear or
/// disappear purely because the backend swapped.
const EFFERENT_WARNING: u32 = 15;
const EFFERENT_ERROR: u32 = 30;

/// Runs the HIR-backed Ce walker. Returns no measurements when the
/// workspace fails to load (e.g. no Cargo.toml, ra_ap_load_cargo
/// errors); the AST-based fallback would too, since the cargo
/// metadata step is shared. Caller falls through to the
/// `unused`-detector's per-file degradation.
pub(super) fn run(workspace_root: &Path, files: &[DiscoveredFile]) -> CrossFilePass {
    let raw = match rustics_ra::metrics::efferent_coupling::detect_at(workspace_root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "rustics: HIR efferent-coupling could not load workspace at {}: {e:#}",
                workspace_root.display()
            );
            return CrossFilePass::default();
        }
    };
    let workspace_prefix = workspace_root.to_string_lossy().into_owned();
    let mut measurements: Vec<MeasurementRecord> = Vec::with_capacity(raw.len());
    let mut violations: Vec<Violation> = Vec::new();
    for m in raw {
        let relative = strip_workspace_prefix(&m.file, &workspace_prefix);
        // Skip files that aren't in the discovered set — the
        // `--exclude` patterns in `rustics.toml` and the `--since`
        // filter apply at the discovery stage, so a measurement
        // whose file path falls outside the discovered set should
        // not surface in the report either.
        if !discovered_match(files, &relative) {
            continue;
        }
        measurements.push(MeasurementRecord {
            file: relative.clone(),
            scope: m.scope.clone(),
            metric: "efferent-coupling".into(),
            value: f64::from(m.value),
        });
        if let Some((severity, threshold)) =
            super::severity_for(m.value, EFFERENT_WARNING, EFFERENT_ERROR)
        {
            violations.push(violation(&relative, &m.scope, m.value, severity, threshold));
        }
    }
    CrossFilePass {
        violations,
        measurements,
    }
}

/// Maps an HIR-reported absolute VFS path to the workspace-relative
/// path the report contract expects.
fn strip_workspace_prefix(file: &str, workspace_prefix: &str) -> String {
    file.strip_prefix(workspace_prefix)
        .map(|s| s.trim_start_matches('/').to_string())
        .unwrap_or_else(|| file.to_string())
}

/// True iff `relative` matches one of the discovered files'
/// `relative` strings. Used to filter out HIR-reported measurements
/// for files that the user excluded via `rustics.toml` or
/// `--since`.
fn discovered_match(files: &[DiscoveredFile], relative: &str) -> bool {
    files.iter().any(|f| f.relative == relative)
}

fn violation(
    relative: &str,
    scope: &str,
    value: u32,
    severity: MetricSeverity,
    threshold: u32,
) -> Violation {
    let id = violation_id(relative, scope, "efferent-coupling");
    Violation {
        id,
        file: relative.to_string(),
        line: 1,
        scope: scope.to_string(),
        scope_kind: ScopeKind::Module,
        metric: "efferent-coupling".into(),
        value: f64::from(value),
        threshold: f64::from(threshold),
        severity,
        rationale: Some(format!(
            "This file reaches {value} distinct external crates via its `use` statements; \
             past the warning threshold of {EFFERENT_WARNING}, the module is likely \
             pulling in more responsibilities than it cleanly owns."
        )),
        refactor_hints: vec![
            "If a file imports many crates, see whether some imports could move to a sibling \
             module that uses them more centrally."
                .to_string(),
        ],
        references: vec![
            "Martin, R. (1994). OO Design Quality Metrics: An Analysis of Dependencies."
                .to_string(),
        ],
        rust_context: Default::default(),
        complexity_justified: None,
    }
}
