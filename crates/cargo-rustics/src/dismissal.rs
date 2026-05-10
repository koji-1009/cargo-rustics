//! Dismissal — "this violation is fine, here's why".
//!
//! Two surfaces:
//!
//! * **Sidecar `.rustics-dismissals.toml`** — file at the workspace
//!   root listing `[[dismissals]]` entries. Source-controlled.
//! * **Doc-comment** — `/// rustics:dismiss <metric> reason="..."` on
//!   the function (or item) being dismissed. Co-located with the
//!   code so an AI agent reading the function sees the reason
//!   without a separate file lookup.
//!
//! Both surfaces flow through the same [`DismissalIndex`]; the merge
//! happens in [`merge_with_sidecar`].
//!
//! Validation rules:
//!
//! * `require_reason: true` (default). A dismissal whose reason is
//!   shorter than `min_reason_length` (default 20) is *rejected* —
//!   the violation stays live and a `dismissalRejected` warning is
//!   emitted.
//! * Entry that does not match any live violation by
//!   `(file, scope, metric)` is *stale* — it stays in the source
//!   (sidecar TOML or doc-comment) but the report's
//!   `staleDismissals:` block lists it.
//! * **Doc-comment + sidecar collision — sidecar wins.** The
//!   sidecar is source-controlled and reviewed at PR time, so we
//!   prefer it over an in-source comment that may have drifted.
//!
//! `--strict-dismiss` (CLI flag) suppresses every dismissal
//! regardless of validity. Useful in CI / final-review mode.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use ra_ap_syntax::{
    ast::{self, HasDocComments, HasModuleItem, HasName},
    Edition, SourceFile,
};
use serde::{Deserialize, Serialize};

use crate::discover::DiscoveredFile;
use crate::report::Violation;

/// File on disk: `.rustics-dismissals.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DismissalsFile {
    /// `[[dismissals]]` entries in source order.
    #[serde(default)]
    pub dismissals: Vec<Dismissal>,
}

/// One dismissal record.
///
/// — `file`, `scope`, `metric` together identify the
/// violation; `reason` documents the call. `by` and `at` are the
/// audit trail; both are optional but encouraged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dismissal {
    /// Workspace-relative file path with `/` separators.
    pub file: String,
    /// Scope path (`module::Type::method`).
    pub scope: String,
    /// Metric id (`cyclomatic-complexity`).
    pub metric: String,
    /// Free-form reason. Plan default: ≥ 20 chars.
    pub reason: String,
    /// Author handle (e.g. `claude-opus-4-7`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// ISO-8601 timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// Configuration knobs for dismissal validation.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct DismissalRules {
    /// Reject dismissals whose `reason` is missing or below the
    /// minimum length.
    #[serde(default = "default_require_reason")]
    pub require_reason: bool,
    /// Minimum acceptable `reason` length when `require_reason` is on.
    #[serde(default = "default_min_reason_length")]
    pub min_reason_length: usize,
    /// Emit a `staleDismissals:` block for sidecar entries that do
    /// not match any live violation.
    #[serde(default = "default_warn_stale")]
    pub warn_stale: bool,
}

fn default_require_reason() -> bool {
    true
}

fn default_min_reason_length() -> usize {
    20
}

fn default_warn_stale() -> bool {
    true
}

impl Default for DismissalRules {
    fn default() -> Self {
        Self {
            require_reason: default_require_reason(),
            min_reason_length: default_min_reason_length(),
            warn_stale: default_warn_stale(),
        }
    }
}

/// Loads `.rustics-dismissals.toml` from `workspace_root` if present.
///
/// Missing file is not an error — most projects do not yet have one,
/// and `dismiss` is opt-in.
pub fn load_sidecar(workspace_root: &Path) -> Result<DismissalsFile> {
    let path = workspace_root.join(".rustics-dismissals.toml");
    if !path.is_file() {
        return Ok(DismissalsFile::default());
    }
    let bytes =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let file: DismissalsFile =
        toml::from_str(&bytes).with_context(|| format!("parse {}", path.display()))?;
    Ok(file)
}

/// Indexed dismissal set with hit-tracking for stale detection.
pub struct DismissalIndex<'a> {
    entries: Vec<DismissalEntry<'a>>,
    rules: DismissalRules,
    strict: bool,
}

struct DismissalEntry<'a> {
    dismissal: &'a Dismissal,
    valid: bool,
    rejection_reason: Option<&'static str>,
    used: std::cell::Cell<bool>,
}

impl<'a> DismissalIndex<'a> {
    /// Builds an index from a sidecar file and the rules.
    pub fn new(file: &'a DismissalsFile, rules: DismissalRules, strict: bool) -> Self {
        let entries = file
            .dismissals
            .iter()
            .map(|d| DismissalEntry::new(d, &rules))
            .collect();
        Self {
            entries,
            rules,
            strict,
        }
    }

    /// Returns whether any *valid* dismissal matches this violation.
    /// Marks the matching entry as used.
    pub fn matches(&self, v: &Violation) -> bool {
        if self.strict {
            return false;
        }
        for entry in &self.entries {
            if !entry.valid {
                continue;
            }
            if entry_matches(entry.dismissal, v) {
                entry.used.set(true);
                return true;
            }
        }
        false
    }

    /// Sidecar entries that were rejected (reason missing / too short).
    /// — these become `dismissalRejected` records in the report.
    pub fn rejected(&self) -> Vec<DismissalRejection<'_>> {
        self.entries
            .iter()
            .filter_map(|e| {
                e.rejection_reason.map(|r| DismissalRejection {
                    dismissal: e.dismissal,
                    reason: r,
                })
            })
            .collect()
    }

    /// Sidecar entries that did not match any live violation.
    /// — these become the `staleDismissals:` block.
    pub fn stale(&self) -> Vec<&Dismissal> {
        if !self.rules.warn_stale {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.valid && !e.used.get())
            .map(|e| e.dismissal)
            .collect()
    }
}

impl<'a> DismissalEntry<'a> {
    fn new(d: &'a Dismissal, rules: &DismissalRules) -> Self {
        let rejection = if rules.require_reason && d.reason.trim().len() < rules.min_reason_length {
            Some("reason too short")
        } else {
            None
        };
        Self {
            dismissal: d,
            valid: rejection.is_none(),
            rejection_reason: rejection,
            used: std::cell::Cell::new(false),
        }
    }
}

fn entry_matches(d: &Dismissal, v: &Violation) -> bool {
    d.file == v.file && d.scope == v.scope && d.metric == v.metric
}

/// Walks every `.rs` file under `files` and collects `///
/// rustics:dismiss <metric> reason="..."` directives attached to
/// items via doc-comment attributes. The scope path matches what the
/// metric pipeline emits — `<file_module_prefix>::<inline_path>::<name>`
/// — so the resulting [`Dismissal`]s flow through [`DismissalIndex`]
/// alongside sidecar entries unchanged.
///
/// Items the parser inspects:
///
/// * top-level `fn` (free function)
/// * `fn` inside an inherent `impl` block (method) and inside a
///   `trait` definition
/// * `struct` / `enum` / `trait` / `impl Foo {...}` blocks (so
///   class-level lenses like LCOM4 / WMC can be dismissed at the
///   type's docstring)
///
/// Items with no doc-comment, or doc-comments that don't carry the
/// directive, contribute zero entries.
pub fn collect_doc_dismissals(files: &[DiscoveredFile]) -> Result<Vec<Dismissal>> {
    let mut out = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file.absolute)
            .with_context(|| format!("read {} for doc-dismissals", file.relative))?;
        let parsed = SourceFile::parse(&source, Edition::CURRENT);
        let module_prefix = file_to_module_prefix(&file.relative);
        collect_in_items(
            &file.relative,
            &module_prefix,
            &[],
            parsed.tree().items(),
            &mut out,
        );
    }
    Ok(out)
}

/// Combines sidecar dismissals with the freshly-collected doc-comment
/// set. On `(file, scope, metric)` collision the sidecar wins; the
/// doc-comment entry is silently dropped because the sidecar is the
/// source-controlled, PR-reviewed surface and we don't want an
/// in-source comment to override it.
///
/// Takes the sidecar by value — the caller already owns the parsed
/// file and the merged result extends it, so consuming avoids a
/// cargo-rustics-flagged clone.
pub fn merge_with_sidecar(
    mut sidecar: DismissalsFile,
    doc_dismissals: Vec<Dismissal>,
) -> DismissalsFile {
    let sidecar_keys: HashSet<(String, String, String)> =
        sidecar.dismissals.iter().map(dismiss_key).collect();
    for d in doc_dismissals {
        if !sidecar_keys.contains(&dismiss_key(&d)) {
            sidecar.dismissals.push(d);
        }
    }
    sidecar
}

fn dismiss_key(d: &Dismissal) -> (String, String, String) {
    (d.file.clone(), d.scope.clone(), d.metric.clone())
}

/// Recursive walk over an `ast::Item` iterator. `parent_path` carries
/// the in-file scope chain accumulated so far (`["mod_a", "Foo"]`
/// for a method inside `mod mod_a { impl Foo { ... } }`). The iterator
/// shape (rather than a slice) matches `HasModuleItem::items`'s
/// `AstChildren<Item>` and `ItemList::items`'s same return type.
fn collect_in_items(
    file: &str,
    module_prefix: &str,
    parent_path: &[String],
    items: impl Iterator<Item = ast::Item>,
    out: &mut Vec<Dismissal>,
) {
    for item in items {
        match item {
            ast::Item::Fn(i) => {
                if let Some(name) = i.name() {
                    emit(file, module_prefix, parent_path, &i, &name.text(), out);
                }
            }
            ast::Item::Struct(i) => {
                if let Some(name) = i.name() {
                    emit(file, module_prefix, parent_path, &i, &name.text(), out);
                }
            }
            ast::Item::Enum(i) => {
                if let Some(name) = i.name() {
                    emit(file, module_prefix, parent_path, &i, &name.text(), out);
                }
            }
            ast::Item::Trait(i) => collect_trait(file, module_prefix, parent_path, &i, out),
            ast::Item::Impl(i) if i.trait_().is_none() => {
                collect_impl(file, module_prefix, parent_path, &i, out);
            }
            ast::Item::Module(m) => {
                if let (Some(name), Some(items)) = (m.name(), m.item_list()) {
                    let mut nested = parent_path.to_vec();
                    nested.push(name.text().to_string());
                    collect_in_items(file, module_prefix, &nested, items.items(), out);
                }
            }
            _ => {}
        }
    }
}

fn collect_trait(
    file: &str,
    module_prefix: &str,
    parent_path: &[String],
    item: &ast::Trait,
    out: &mut Vec<Dismissal>,
) {
    let Some(name) = item.name() else { return };
    let name_text = name.text().to_string();
    emit(file, module_prefix, parent_path, item, &name_text, out);
    let mut nested = parent_path.to_vec();
    nested.push(name_text);
    let Some(list) = item.assoc_item_list() else {
        return;
    };
    for ai in list.assoc_items() {
        if let ast::AssocItem::Fn(f) = ai {
            if let Some(fname) = f.name() {
                emit(file, module_prefix, &nested, &f, &fname.text(), out);
            }
        }
    }
}

fn collect_impl(
    file: &str,
    module_prefix: &str,
    parent_path: &[String],
    item: &ast::Impl,
    out: &mut Vec<Dismissal>,
) {
    let parent_name = item.self_ty().as_ref().and_then(type_path_last_segment);
    if let Some(name) = parent_name.as_deref() {
        emit(file, module_prefix, parent_path, item, name, out);
    }
    let mut nested = parent_path.to_vec();
    if let Some(name) = parent_name {
        nested.push(name);
    }
    let Some(list) = item.assoc_item_list() else {
        return;
    };
    for ai in list.assoc_items() {
        if let ast::AssocItem::Fn(f) = ai {
            if let Some(fname) = f.name() {
                emit(file, module_prefix, &nested, &f, &fname.text(), out);
            }
        }
    }
}

/// Pulls every `rustics:dismiss` directive from `node`'s doc comments
/// and pushes a [`Dismissal`] for each one. Builds the scope path by
/// joining `module_prefix`, `parent_path`, and `name` with `::`.
fn emit(
    file: &str,
    module_prefix: &str,
    parent_path: &[String],
    node: &dyn HasDocComments,
    name: &str,
    out: &mut Vec<Dismissal>,
) {
    let directives = parse_directives(node);
    if directives.is_empty() {
        return;
    }
    let mut scope_parts: Vec<&str> = Vec::with_capacity(parent_path.len() + 2);
    if !module_prefix.is_empty() {
        scope_parts.push(module_prefix);
    }
    for p in parent_path {
        scope_parts.push(p.as_str());
    }
    scope_parts.push(name);
    let scope = scope_parts.join("::");
    for (metric, reason) in directives {
        out.push(Dismissal {
            file: file.to_string(),
            scope: scope.clone(),
            metric,
            reason,
            by: None,
            at: None,
        });
    }
}

/// Returns every `(metric, reason)` directive found on `node`'s doc
/// comments. A single item can stack multiple `///` lines; each line
/// surfaces as a separate `ast::Comment` and is parsed in isolation.
fn parse_directives(node: &dyn HasDocComments) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for comment in node.doc_comments() {
        let Some((text, _offset)) = comment.doc_comment() else {
            continue;
        };
        if let Some(d) = parse_directive_line(text) {
            out.push(d);
        }
    }
    out
}

/// Parses a single doc-comment line for the dismiss directive.
/// Format: `rustics:dismiss <metric-id> reason="<text>"`. Returns
/// `None` for any line that doesn't match — this is purposely strict
/// so a docstring that *mentions* the directive in prose doesn't
/// accidentally fire:
///
/// * `<metric-id>` must be kebab-case `[a-z0-9-]+` (real metric ids
///   never contain `<` / `>` / `_` / spaces). Placeholder text like
///   `<metric>` in a syntax-explanation docstring fails this check.
/// * Anything other than whitespace after the closing `"` of the
///   reason clause invalidates the line — a docstring that wraps
///   the syntax in prose ("`rustics:dismiss …` directives are…")
///   has trailing words and is rejected.
fn parse_directive_line(line: &str) -> Option<(String, String)> {
    let s = line.trim().strip_prefix("rustics:dismiss")?.trim_start();
    let (metric, rest) = split_first_token(s)?;
    if !is_valid_metric_id(metric) {
        return None;
    }
    let reason = parse_reason_clause(rest.trim_start())?;
    Some((metric.to_string(), reason))
}

/// Pulls the `reason="<text>"` clause from `rest`. Requires the
/// closing `"` to be the end of the meaningful line — anything other
/// than whitespace after it invalidates the directive.
fn parse_reason_clause(rest: &str) -> Option<String> {
    let body = rest.strip_prefix("reason=")?.strip_prefix('"')?;
    let close = body.find('"')?;
    if !body[close + 1..].trim().is_empty() {
        return None;
    }
    Some(body[..close].to_string())
}

fn is_valid_metric_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn split_first_token(s: &str) -> Option<(&str, &str)> {
    let end = s.find(char::is_whitespace)?;
    if end == 0 {
        return None;
    }
    Some((&s[..end], &s[end..]))
}

/// Last segment of a path-typed self type. `impl Foo<T>` → `Foo`.
/// Returns `None` for tuple / reference / fn-pointer self types. The
/// match shape mirrors `cross_file::trait_impl_fanout::type_name` —
/// PathType / RefType / ParenType are the cases an inherent-impl self
/// type can take in real code; everything else (tuples, fn-pointers,
/// arrays) cannot be the receiver of a doc-dismissable inherent impl.
fn type_path_last_segment(ty: &ast::Type) -> Option<String> {
    match ty {
        ast::Type::PathType(p) => p
            .path()
            .and_then(|p| p.segment())
            .and_then(|s| s.name_ref())
            .map(|n| n.text().to_string()),
        ast::Type::RefType(r) => r.ty().as_ref().and_then(type_path_last_segment),
        ast::Type::ParenType(p) => p.ty().as_ref().and_then(type_path_last_segment),
        _ => None,
    }
}

/// Mirrors the analyzer's file-path → module-path derivation so the
/// scope strings produced here match the violation's `scope:` field.
/// `crates/foo/src/baz/qux.rs` → `baz::qux`; `src/lib.rs` → `""`.
fn file_to_module_prefix(relative: &str) -> String {
    let path = std::path::Path::new(relative);
    let mut after_src: Vec<String> = path
        .iter()
        .skip_while(|p| p.to_str() != Some("src"))
        .skip(1)
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if let Some(last) = after_src.last_mut() {
        if let Some(stripped) = last.strip_suffix(".rs") {
            *last = stripped.to_string();
        }
    }
    if matches!(
        after_src.last().map(String::as_str),
        Some("lib" | "main" | "mod")
    ) {
        after_src.pop();
    }
    after_src.join("::")
}

/// Display row for `dismissalRejected:` block.
pub struct DismissalRejection<'a> {
    /// The original sidecar entry.
    pub dismissal: &'a Dismissal,
    /// One-line rejection reason.
    pub reason: &'static str,
}

#[cfg(test)]
mod tests {
    static TEMPDIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    use super::*;
    use crate::report::Summary;
    use rustics::{MetricSeverity, ScopeKind};

    fn dismissal(file: &str, scope: &str, metric: &str, reason: &str) -> Dismissal {
        Dismissal {
            file: file.into(),
            scope: scope.into(),
            metric: metric.into(),
            reason: reason.into(),
            by: None,
            at: None,
        }
    }

    fn violation(file: &str, scope: &str, metric: &str) -> Violation {
        Violation {
            id: "abc".into(),
            file: file.into(),
            line: 1,
            scope: scope.into(),
            scope_kind: ScopeKind::FreeFunction,
            metric: metric.into(),
            value: 11.0,
            threshold: 10.0,
            severity: MetricSeverity::Warning,
            rationale: None,
            refactor_hints: vec![],
            references: vec![],
            rust_context: Default::default(),
            complexity_justified: None,
        }
    }

    #[test]
    fn matching_dismissal_filters_violation() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/x.rs",
                "f",
                "cyclomatic-complexity",
                "twenty character reason here",
            )],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        assert!(idx.matches(&violation("src/x.rs", "f", "cyclomatic-complexity")));
    }

    #[test]
    fn unmatched_violation_passes_through() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/other.rs",
                "f",
                "cyclomatic-complexity",
                "twenty character reason here",
            )],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        assert!(!idx.matches(&violation("src/x.rs", "f", "cyclomatic-complexity")));
    }

    #[test]
    fn short_reason_is_rejected() {
        let file = DismissalsFile {
            dismissals: vec![dismissal("src/x.rs", "f", "cc", "short")],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        assert!(!idx.matches(&violation("src/x.rs", "f", "cc")));
        let rejected = idx.rejected();
        assert_eq!(rejected.len(), 1);
        assert!(rejected[0].reason.contains("too short"));
    }

    #[test]
    fn stale_dismissal_is_reported() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/old.rs",
                "ghost",
                "cc",
                "twenty character reason here",
            )],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        // No matches.
        assert_eq!(idx.stale().len(), 1);
    }

    #[test]
    fn strict_mode_skips_all_dismissals() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/x.rs",
                "f",
                "cc",
                "twenty character reason here",
            )],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), /* strict */ true);
        assert!(!idx.matches(&violation("src/x.rs", "f", "cc")));
    }

    #[test]
    fn helper_summary_for_test_coverage() {
        // Touch the Summary type via a default to keep the test crate's
        // unused-import warnings quiet when this module runs solo.
        let _ = Summary {
            files_analyzed: 0,
            violations: 0,
            warnings: 0,
            errors: 0,
            warnings_justified: 0,
            errors_justified: 0,
        };
    }

    #[test]
    fn defaults_match_plan_documented_values() {
        let r = DismissalRules::default();
        assert!(r.require_reason);
        assert_eq!(r.min_reason_length, 20);
        assert!(r.warn_stale);
    }

    #[test]
    fn warn_stale_false_returns_no_stale() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/old.rs",
                "ghost",
                "cc",
                "twenty character reason here",
            )],
        };
        let rules = DismissalRules {
            warn_stale: false,
            ..Default::default()
        };
        let idx = DismissalIndex::new(&file, rules, false);
        assert!(idx.stale().is_empty());
    }

    #[test]
    fn used_dismissal_is_not_stale() {
        let file = DismissalsFile {
            dismissals: vec![dismissal(
                "src/x.rs",
                "f",
                "cc",
                "twenty character reason here",
            )],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        // Match it once.
        assert!(idx.matches(&violation("src/x.rs", "f", "cc")));
        // Now it's used → not stale.
        assert!(idx.stale().is_empty());
    }

    #[test]
    fn invalid_dismissal_does_not_count_toward_stale() {
        let file = DismissalsFile {
            dismissals: vec![dismissal("src/x.rs", "f", "cc", "short")],
        };
        let idx = DismissalIndex::new(&file, DismissalRules::default(), false);
        // The entry is invalid (rejected) → it's not stale, it's
        // rejected; rejected and stale are disjoint sets.
        assert!(idx.stale().is_empty());
        assert_eq!(idx.rejected().len(), 1);
    }

    fn write_workspace_with_sidecar(toml: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMPDIR_SEQ.fetch_add(
            1,
            std::sync::atomic::Ordering::Relaxed,
        );
        let dir = std::env::temp_dir().join(format!("rustics-dismiss-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&dir).unwrap();
        if !toml.is_empty() {
            std::fs::write(dir.join(".rustics-dismissals.toml"), toml).unwrap();
        }
        dir
    }

    #[test]
    fn load_sidecar_returns_default_when_absent() {
        let dir = write_workspace_with_sidecar("");
        let f = load_sidecar(&dir).unwrap();
        assert!(f.dismissals.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_sidecar_parses_toml() {
        let dir = write_workspace_with_sidecar(
            r#"[[dismissals]]
file = "src/x.rs"
scope = "f"
metric = "cyclomatic-complexity"
reason = "twenty character reason here"
"#,
        );
        let f = load_sidecar(&dir).unwrap();
        assert_eq!(f.dismissals.len(), 1);
        assert_eq!(f.dismissals[0].file, "src/x.rs");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_sidecar_errors_on_invalid_toml() {
        let dir = write_workspace_with_sidecar("[[ malformed\n");
        let err = load_sidecar(&dir).unwrap_err();
        assert!(format!("{err:#}").contains("parse"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------
    // Doc-comment channel.
    // -----------------------------------------------------------------

    fn write_doc_file(rel: &str, body: &str) -> (DiscoveredFile, std::path::PathBuf) {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMPDIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rustics-doc-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&dir).unwrap();
        let abs = dir.join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, body).unwrap();
        (
            DiscoveredFile {
                absolute: abs,
                relative: rel.to_string(),
            },
            dir,
        )
    }

    #[test]
    fn parse_directive_line_extracts_metric_and_reason() {
        let line =
            r#" rustics:dismiss cyclomatic-complexity reason="Twenty-character reason here." "#;
        let (m, r) = parse_directive_line(line).expect("parsed");
        assert_eq!(m, "cyclomatic-complexity");
        assert_eq!(r, "Twenty-character reason here.");
    }

    #[test]
    fn parse_directive_line_rejects_lines_without_directive_prefix() {
        // A docstring that *mentions* the directive in prose must
        // not fire. The strict prefix match is intentional.
        let line = " See `rustics:dismiss <metric>` for syntax.";
        assert!(parse_directive_line(line).is_none());
    }

    #[test]
    fn parse_directive_line_rejects_missing_reason_clause() {
        assert!(parse_directive_line("rustics:dismiss cyclomatic-complexity").is_none());
        assert!(parse_directive_line(r#"rustics:dismiss cyclomatic-complexity reason=foo"#).is_none());
        assert!(
            parse_directive_line(r#"rustics:dismiss cyclomatic-complexity reason="unterminated"#)
                .is_none()
        );
    }

    #[test]
    fn parse_directive_line_handles_immediate_whitespace_after_prefix() {
        // No metric token after the prefix — must not crash, must return None.
        assert!(parse_directive_line("rustics:dismiss   ").is_none());
    }

    #[test]
    fn parse_directive_line_rejects_placeholder_metric_id() {
        // A docstring describing the syntax with `<metric>` placeholder
        // must not be picked up as a real directive.
        assert!(
            parse_directive_line(r#"rustics:dismiss <metric> reason="example reason here..." "#)
                .is_none()
        );
        assert!(
            parse_directive_line(r#"rustics:dismiss METRIC_ID reason="example reason here..."  "#)
                .is_none()
        );
    }

    #[test]
    fn parse_directive_line_rejects_trailing_prose() {
        // A docstring that wraps the syntax in prose ("`rustics:dismiss
        // …` directives are…") must not be picked up either, even
        // when the metric id passes the kebab-case check.
        let line = r#"rustics:dismiss cyclomatic-complexity reason="ok"` directives are then…"#;
        assert!(parse_directive_line(line).is_none());
    }

    #[test]
    fn is_valid_metric_id_accepts_kebab_case_only() {
        assert!(is_valid_metric_id("cyclomatic-complexity"));
        assert!(is_valid_metric_id("lcom4"));
        assert!(is_valid_metric_id("a"));
        assert!(!is_valid_metric_id(""));
        assert!(!is_valid_metric_id("UPPERCASE"));
        assert!(!is_valid_metric_id("snake_case"));
        assert!(!is_valid_metric_id("<metric>"));
        assert!(!is_valid_metric_id("with space"));
    }

    #[test]
    fn collect_finds_top_level_fn_directive() {
        let (file, dir) = write_doc_file(
            "src/lib.rs",
            "/// First doc line.\n\
             /// rustics:dismiss cyclomatic-complexity reason=\"State machine: branching is intent.\"\n\
             pub fn parse() {}\n",
        );
        let out = collect_doc_dismissals(&[file]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].file, "src/lib.rs");
        assert_eq!(out[0].scope, "parse");
        assert_eq!(out[0].metric, "cyclomatic-complexity");
        assert!(out[0].reason.contains("State machine"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_handles_stacked_directives_on_one_item() {
        let (file, dir) = write_doc_file(
            "src/lib.rs",
            "/// rustics:dismiss cyclomatic-complexity reason=\"first reason; long enough.\"\n\
             /// rustics:dismiss method-length reason=\"second reason; long enough.\"\n\
             pub fn parse() {}\n",
        );
        let out = collect_doc_dismissals(&[file]).unwrap();
        assert_eq!(out.len(), 2);
        let metrics: Vec<&str> = out.iter().map(|d| d.metric.as_str()).collect();
        assert!(metrics.contains(&"cyclomatic-complexity"));
        assert!(metrics.contains(&"method-length"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_builds_module_prefix_from_file_path() {
        let (file, dir) = write_doc_file(
            "crates/foo/src/parser/lexer.rs",
            "/// rustics:dismiss cyclomatic-complexity reason=\"Long enough reason here.\"\n\
             pub fn lex() {}\n",
        );
        let out = collect_doc_dismissals(&[file]).unwrap();
        assert_eq!(out.len(), 1);
        // file_to_module_prefix("crates/foo/src/parser/lexer.rs") = "parser::lexer"
        // joined with the fn name = "parser::lexer::lex".
        assert_eq!(out[0].scope, "parser::lexer::lex");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_builds_impl_method_scope() {
        let (file, dir) = write_doc_file(
            "src/lib.rs",
            "pub struct Foo;\n\
             impl Foo {\n\
                 /// rustics:dismiss cyclomatic-complexity reason=\"Long enough reason here.\"\n\
                 pub fn run(&self) {}\n\
             }\n",
        );
        let out = collect_doc_dismissals(&[file]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].scope, "Foo::run");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_builds_class_level_scope() {
        // class-level lenses (LCOM4, WMC) target the type itself; the
        // dismiss directive on `pub struct` should produce a scope of
        // just the type's name.
        let (file, dir) = write_doc_file(
            "src/lib.rs",
            "/// rustics:dismiss lcom4 reason=\"Concept is intentionally split.\"\n\
             pub struct Foo;\n",
        );
        let out = collect_doc_dismissals(&[file]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].scope, "Foo");
        assert_eq!(out[0].metric, "lcom4");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_recurses_into_inline_modules() {
        let (file, dir) = write_doc_file(
            "src/lib.rs",
            "pub mod inner {\n\
                 /// rustics:dismiss cyclomatic-complexity reason=\"Long enough reason here.\"\n\
                 pub fn deep() {}\n\
             }\n",
        );
        let out = collect_doc_dismissals(&[file]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].scope, "inner::deep");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_picks_up_trait_method_directive() {
        let (file, dir) = write_doc_file(
            "src/lib.rs",
            "pub trait Parser {\n\
                 /// rustics:dismiss cyclomatic-complexity reason=\"Long enough reason here.\"\n\
                 fn parse(&self);\n\
             }\n",
        );
        let out = collect_doc_dismissals(&[file]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].scope, "Parser::parse");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn collect_skips_files_that_fail_to_parse() {
        let (file, dir) = write_doc_file("src/lib.rs", "this is :: not :: rust\n");
        let out = collect_doc_dismissals(&[file]).unwrap();
        assert!(out.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_with_sidecar_is_no_op_when_no_doc_dismissals() {
        let sidecar = DismissalsFile {
            dismissals: vec![dismissal("src/x.rs", "f", "cc", "long enough reason here")],
        };
        let merged = merge_with_sidecar(sidecar, vec![]);
        assert_eq!(merged.dismissals.len(), 1);
    }

    #[test]
    fn merge_with_sidecar_appends_doc_only_entries() {
        let sidecar = DismissalsFile {
            dismissals: vec![dismissal("src/x.rs", "f", "cc", "long enough reason here")],
        };
        let merged = merge_with_sidecar(
            sidecar,
            vec![dismissal("src/y.rs", "g", "cc", "different long reason here")],
        );
        assert_eq!(merged.dismissals.len(), 2);
    }

    #[test]
    fn merge_with_sidecar_drops_doc_entry_on_collision() {
        // Same (file, scope, metric) → sidecar wins; doc entry
        // silently dropped.
        let sidecar = DismissalsFile {
            dismissals: vec![dismissal("src/x.rs", "f", "cc", "sidecar reason long enough")],
        };
        let merged = merge_with_sidecar(
            sidecar,
            vec![dismissal("src/x.rs", "f", "cc", "doc reason long enough.")],
        );
        assert_eq!(merged.dismissals.len(), 1);
        assert!(merged.dismissals[0].reason.contains("sidecar"));
    }

    #[test]
    fn doc_dismissal_filters_violation_via_index() {
        // End-to-end: collect from source, merge with empty sidecar,
        // build index, and assert the violation is dismissed.
        let (file, dir) = write_doc_file(
            "src/lib.rs",
            "/// rustics:dismiss cyclomatic-complexity reason=\"Long enough reason here.\"\n\
             pub fn parse() {}\n",
        );
        let docs = collect_doc_dismissals(&[file]).unwrap();
        let merged = merge_with_sidecar(DismissalsFile::default(), docs);
        let idx = DismissalIndex::new(&merged, DismissalRules::default(), false);
        let v = violation("src/lib.rs", "parse", "cyclomatic-complexity");
        assert!(idx.matches(&v));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doc_dismissal_short_reason_is_rejected() {
        let (file, dir) = write_doc_file(
            "src/lib.rs",
            "/// rustics:dismiss cyclomatic-complexity reason=\"too short\"\n\
             pub fn parse() {}\n",
        );
        let docs = collect_doc_dismissals(&[file]).unwrap();
        let merged = merge_with_sidecar(DismissalsFile::default(), docs);
        let idx = DismissalIndex::new(&merged, DismissalRules::default(), false);
        // Reason is below 20 chars → rejected; violation passes through live.
        assert!(!idx.matches(&violation(
            "src/lib.rs",
            "parse",
            "cyclomatic-complexity"
        )));
        assert_eq!(idx.rejected().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Test helper: parse a single `Type` by wrapping it in a type
    /// alias and pulling the type back out. ra_ap_syntax does not
    /// expose a "parse this fragment as a type" helper the way `syn`
    /// does, so the wrap-then-extract round-trip is the canonical
    /// pattern (also used in `cross_file::trait_impl_fanout::tests`).
    fn parse_type(s: &str) -> ast::Type {
        use ra_ap_syntax::AstNode as _;
        let src = format!("type _X = {s};");
        let parsed = SourceFile::parse(&src, Edition::CURRENT);
        parsed
            .tree()
            .syntax()
            .descendants()
            .filter_map(ast::TypeAlias::cast)
            .next()
            .and_then(|ta| ta.ty())
            .expect("parse_type")
    }

    #[test]
    fn type_path_last_segment_returns_none_for_tuple_self() {
        let ty = parse_type("(u8, u8)");
        assert!(type_path_last_segment(&ty).is_none());
        let ty = parse_type("Foo<u8>");
        assert_eq!(type_path_last_segment(&ty).as_deref(), Some("Foo"));
    }

    #[test]
    fn file_to_module_prefix_strips_lib_main_mod() {
        assert_eq!(file_to_module_prefix("crates/foo/src/lib.rs"), "");
        assert_eq!(file_to_module_prefix("crates/foo/src/main.rs"), "");
        assert_eq!(file_to_module_prefix("crates/foo/src/baz/mod.rs"), "baz");
        assert_eq!(
            file_to_module_prefix("crates/foo/src/baz/qux.rs"),
            "baz::qux"
        );
    }
}
