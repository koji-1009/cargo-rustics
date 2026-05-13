//! Public-API reachability — name-resolution-aware detector backed
//! by `ra_ap_hir`.
//!
//! `cargo rustics analyze` and `cargo rustics unused` both call into
//! `detect_at`, which loads the cargo workspace via
//! `ra_ap_load_cargo`, walks every workspace-local definition, and
//! emits an [`UnusedItem`] for each `pub` declaration whose HIR
//! reference set (excluding `pub use` re-exports) is empty.
//!
//! Scope of the heuristic — what's covered, what's not:
//!
//! * **Declarations covered.** Top-level `pub` `fn` / `struct` /
//!   `enum` / `trait` / `type` / `const` / `static` (every
//!   `ModuleDef` variant `rustics-ra` classifies), plus every
//!   inherent-impl `fn` / `const` (surfaced with `parent` set to the
//!   `impl T { … }` Self type's name). Trait impls are skipped — the
//!   method set there is dictated by the trait contract, not a
//!   cohesion choice on the type.
//! * **References counted.** Every HIR `Definition::usages` hit that
//!   isn't `ReferenceCategory::IMPORT`-flagged. Macro-body method
//!   calls (the `eprintln!("{}", c.method())` case the unexpanded
//!   AST cannot see) resolve correctly because HIR runs the macro
//!   server.
//! * **Roots — known limit.** `fn main`, `#[test]`, `#[bench]`,
//!   `#[no_mangle]`, and similar entry-point attributes are *not*
//!   yet recognised as roots. Binary mains and test functions show
//!   up in the unused report; the caller may need to dismiss them
//!   per project until the root recogniser lands.
//! * **Public API consumed only by external crates.** A `pub fn` in
//!   `lib.rs` that's used by another crate but never referenced
//!   inside this workspace will be flagged. That's by design — for
//!   an AI loop, "no internal user, no test" is a legitimate signal
//!   to confirm the API has a consumer somewhere.
//! * **Serde / proptest string-attr references.** `#[serde(default
//!   = "fn_name")]` and `proptest!` macro-local references resolve
//!   through string literals or third-party macro bodies the HIR
//!   walker cannot follow. Items reached only through those paths
//!   will surface as unused.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub mod apply;

/// One unused-public finding.
///
/// `kind` is one of [`KNOWN_KINDS`]; we keep it `String` for serde
/// round-trips (static-str source values are still funnelled through
/// the `KNOWN_KINDS` constants at construction time).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnusedItem {
    /// Workspace-relative path.
    pub file: String,
    /// 1-based line number of the item declaration.
    pub line: usize,
    /// Item name (`fn` / `struct` / variant / method).
    pub name: String,
    /// Item kind for display (`fn`, `struct`, `enum`, `variant`,
    /// `method`, …). Stable across versions; printed verbatim by
    /// [`format`].
    pub kind: String,
    /// Containing scope. `None` for top-level items, `Some(enum_name)`
    /// for variants, `Some(type_name)` for inherent impl methods /
    /// associated consts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// Helper for crate-level workspace lookups. Loads the cargo
/// workspace once, runs the HIR-backed walker, and converts the
/// `rustics_ra` finding shape into this crate's [`UnusedItem`]
/// (workspace-relative paths, 1-based lines, `String`-typed kind).
///
/// `rustics.toml`'s `[rustics.exclude]` patterns are honoured at the
/// conversion boundary — files matched by `exclude` are filtered
/// before the report is returned.
pub fn detect_at(workspace_root: &Path) -> Result<Vec<UnusedItem>> {
    let config = crate::config::load_config(workspace_root)?;
    let raw = rustics_ra::unused::detect_at(workspace_root)?;
    let workspace_prefix = workspace_root.to_string_lossy().into_owned();
    let mut converted: Vec<UnusedItem> = raw
        .into_iter()
        .map(|hir| convert_hir_item(hir, &workspace_prefix))
        .filter(|item| !config.exclude().matches(&item.file))
        .collect();
    converted.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(converted)
}

/// Maps the HIR detector's `UnusedItem` to this crate's report shape.
/// Three normalisations:
///
/// * `file` arrives from `ra_ap_vfs` as an absolute path with `/`
///   separators (the same shape rust-analyzer carries internally);
///   strip the workspace prefix to match the AI-report contract,
///   which is workspace-relative.
/// * `line` is 0-based in `ra_ap_ide::LineIndex`; the report
///   contract is 1-based.
/// * `kind: &'static str` widens to `String` because the report
///   schema uses `String` for serde round-trips.
fn convert_hir_item(hir: rustics_ra::unused::UnusedItem, workspace_prefix: &str) -> UnusedItem {
    let relative = hir
        .file
        .strip_prefix(workspace_prefix)
        .map(|s| s.trim_start_matches('/').to_string())
        .unwrap_or(hir.file);
    UnusedItem {
        file: relative,
        line: (hir.line as usize).saturating_add(1),
        name: hir.name,
        kind: hir.kind.to_string(),
        parent: hir.parent,
    }
}

/// Every declaration kind the detector emits, in the canonical
/// kebab-case spelling the CLI accepts on `--filter`. Single source of
/// truth for the validator and the tests.
pub const KNOWN_KINDS: &[&str] = &[
    "fn",
    "struct",
    "enum",
    "trait",
    "type",
    "const",
    "static",
    "union",
    "variant",
    "method",
    "assoc-const",
    "adt",
];

/// Validates `--filter` values from the CLI and returns the allow-set.
/// Returns `Ok(None)` when the user passed no filter (default = all
/// kinds). Returns an error on the first unknown kind so a typo
/// (`--filter functon`) doesn't silently drop the entire report.
pub fn parse_kind_filter(values: &[String]) -> Result<Option<HashSet<String>>> {
    if values.is_empty() {
        return Ok(None);
    }
    let mut allowed = HashSet::new();
    for raw in values {
        for chunk in raw.split(',') {
            let kind = chunk.trim();
            if kind.is_empty() {
                continue;
            }
            if !KNOWN_KINDS.contains(&kind) {
                anyhow::bail!(
                    "unused --filter: unknown kind `{kind}`. Valid kinds: {}",
                    KNOWN_KINDS.join(", ")
                );
            }
            allowed.insert(kind.to_string());
        }
    }
    if allowed.is_empty() {
        return Ok(None);
    }
    Ok(Some(allowed))
}

/// Returns the subset of `items` whose kind is in `allowed`. When
/// `allowed` is `None` (no filter), the input is returned unchanged.
pub fn apply_kind_filter(
    items: Vec<UnusedItem>,
    allowed: Option<&HashSet<String>>,
) -> Vec<UnusedItem> {
    let Some(set) = allowed else {
        return items;
    };
    items
        .into_iter()
        .filter(|i| set.contains(&i.kind))
        .collect()
}

/// Renders a small reporter-ish text dump for `cargo rustics unused`.
pub fn format(items: &[UnusedItem]) -> String {
    if items.is_empty() {
        return "rustics unused: no candidates found.\n".to_string();
    }
    let mut s = format!("rustics unused: {} candidate(s):\n", items.len());
    for item in items {
        match &item.parent {
            Some(parent) => s.push_str(&format!(
                "  {kind} {parent}::{name} — {file}:{line}\n",
                kind = item.kind,
                parent = parent,
                name = item.name,
                file = item.file,
                line = item.line,
            )),
            None => s.push_str(&format!(
                "  {kind} {name} — {file}:{line}\n",
                kind = item.kind,
                name = item.name,
                file = item.file,
                line = item.line,
            )),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, name: &str, file: &str, line: usize) -> UnusedItem {
        UnusedItem {
            file: file.into(),
            line,
            name: name.into(),
            kind: kind.into(),
            parent: None,
        }
    }

    #[test]
    fn known_kinds_covers_every_classifier_output() {
        // Sanity check on the catalogue: every kind that
        // `rustics_ra::unused` emits must round-trip through the
        // CLI's --filter validator without tripping the
        // "unknown kind" bail.
        for kind in ["fn", "struct", "method", "assoc-const", "adt", "trait"] {
            assert!(
                KNOWN_KINDS.contains(&kind),
                "kind `{kind}` is emitted by the HIR walker but missing from KNOWN_KINDS"
            );
        }
    }

    #[test]
    fn parse_kind_filter_none_when_empty() {
        assert!(parse_kind_filter(&[]).unwrap().is_none());
    }

    #[test]
    fn parse_kind_filter_accepts_comma_separated() {
        let parsed = parse_kind_filter(&["fn,struct".into()]).unwrap().unwrap();
        assert!(parsed.contains("fn"));
        assert!(parsed.contains("struct"));
    }

    #[test]
    fn parse_kind_filter_rejects_unknown_kind() {
        let err = parse_kind_filter(&["typoeneous".into()]).unwrap_err();
        assert!(format!("{err:#}").contains("unknown kind"));
    }

    #[test]
    fn parse_kind_filter_ignores_only_whitespace_entries() {
        let parsed = parse_kind_filter(&["  , fn ,  ".into()]).unwrap().unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed.contains("fn"));
    }

    #[test]
    fn parse_kind_filter_returns_none_when_every_chunk_empty() {
        // A `--filter ,,,` value passes the outer non-empty check but
        // every chunk is empty after trimming. We must still
        // short-circuit to "no filter" rather than treating it as a
        // filter that matches nothing.
        let parsed = parse_kind_filter(&[",,".into()]).unwrap();
        assert!(parsed.is_none());
    }

    #[test]
    fn apply_kind_filter_returns_input_unchanged_when_no_allowset() {
        let input = vec![item("fn", "a", "a.rs", 1)];
        let out = apply_kind_filter(input.clone(), None);
        assert_eq!(out.len(), input.len());
    }

    #[test]
    fn apply_kind_filter_keeps_only_allowed_kinds() {
        let input = vec![
            item("fn", "a", "a.rs", 1),
            item("struct", "S", "a.rs", 2),
            item("method", "m", "a.rs", 3),
        ];
        let allowed: HashSet<String> = ["fn".to_string(), "method".to_string()]
            .into_iter()
            .collect();
        let out = apply_kind_filter(input, Some(&allowed));
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|i| i.kind == "fn" || i.kind == "method"));
    }

    #[test]
    fn format_handles_empty_list() {
        assert_eq!(format(&[]), "rustics unused: no candidates found.\n");
    }

    #[test]
    fn format_renders_top_level_items_without_parent_prefix() {
        let body = format(&[item("fn", "free", "a.rs", 7)]);
        assert!(body.contains("fn free — a.rs:7"), "got: {body}");
    }

    #[test]
    fn format_renders_method_items_with_parent_prefix() {
        let mut m = item("method", "do_it", "a.rs", 9);
        m.parent = Some("Foo".into());
        let body = format(&[m]);
        assert!(body.contains("method Foo::do_it — a.rs:9"), "got: {body}");
    }

    #[test]
    fn convert_hir_item_strips_workspace_prefix_and_renames_line() {
        let hir = rustics_ra::unused::UnusedItem {
            file: "/ws/crates/foo/src/lib.rs".into(),
            line: 41, // 0-based
            name: "thing".into(),
            kind: "fn",
            parent: None,
        };
        let item = convert_hir_item(hir, "/ws");
        assert_eq!(item.file, "crates/foo/src/lib.rs");
        assert_eq!(item.line, 42); // 1-based
        assert_eq!(item.kind, "fn");
    }

    #[test]
    fn convert_hir_item_passes_path_through_when_no_prefix_match() {
        let hir = rustics_ra::unused::UnusedItem {
            file: "absolute/elsewhere.rs".into(),
            line: 0,
            name: "x".into(),
            kind: "fn",
            parent: None,
        };
        let item = convert_hir_item(hir, "/ws");
        assert_eq!(item.file, "absolute/elsewhere.rs");
        assert_eq!(item.line, 1);
    }
}
