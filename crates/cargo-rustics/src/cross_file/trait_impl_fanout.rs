//! `trait-impl-fanout` — the count of `impl` blocks targeting a
//! single struct/enum across the whole workspace.
//!
//! The per-file lens infrastructure (which underlies
//! every lens) does not see other files; this module fills that
//! gap by re-walking the discovered file set and aggregating impl
//! receivers.

use std::collections::HashMap;

use ra_ap_syntax::{
    ast::{self, AstNode},
    SyntaxNode,
};
use rustics::{violation_id, ScopeKind};

use crate::report::{MeasurementRecord, Violation};

use super::{CrossFilePass, ParsedFile};

/// Threshold defaults — chosen by the same eye that picked the
/// per-impl-block ones.
const TRAIT_IMPL_FANOUT_WARNING: u32 = 8;
const TRAIT_IMPL_FANOUT_ERROR: u32 = 16;

/// Walks every parsed file. Emits one `trait-impl-fanout`
/// measurement per type with at least one impl, and one violation
/// per type whose impl-block count crosses the warning/error
/// threshold. Measurements are emitted regardless of threshold so
/// `regression`'s cosmetic-detection sees fanout drifts (e.g. 6 →
/// 7) that don't yet violate.
pub(super) fn run(parsed: &[ParsedFile]) -> CrossFilePass {
    let mut buckets: HashMap<String, Vec<TypeImplLocation>> = HashMap::new();
    for file in parsed {
        collect_impls(file, &mut buckets);
    }
    CrossFilePass {
        violations: emit_violations(&buckets),
        measurements: emit_measurements(&buckets),
    }
}

#[derive(Debug, Clone)]
struct TypeImplLocation {
    file: String,
    line: usize,
}

fn collect_impls(file: &ParsedFile, out: &mut HashMap<String, Vec<TypeImplLocation>>) {
    let source_text = file.tree.syntax().text().to_string();
    for desc in file.tree.syntax().descendants() {
        let Some(impl_) = ast::Impl::cast(desc) else {
            continue;
        };
        let Some(name) = impl_.self_ty().and_then(|t| type_name(&t)) else {
            continue;
        };
        let line = line_of(&source_text, impl_.syntax());
        out.entry(name).or_default().push(TypeImplLocation {
            file: file.relative.clone(),
            line,
        });
    }
}

fn type_name(ty: &ast::Type) -> Option<String> {
    match ty {
        ast::Type::PathType(p) => {
            p.path().and_then(|p| p.segment()).and_then(|s| s.name_ref()).map(|n| n.text().to_string())
        }
        ast::Type::RefType(r) => r.ty().as_ref().and_then(type_name),
        ast::Type::ParenType(p) => p.ty().as_ref().and_then(type_name),
        _ => None,
    }
}

fn line_of(source: &str, node: &SyntaxNode) -> usize {
    let offset: usize = node.text_range().start().into();
    source.get(..offset).unwrap_or("").bytes().filter(|b| *b == b'\n').count() + 1
}

fn emit_violations(buckets: &HashMap<String, Vec<TypeImplLocation>>) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut sorted: Vec<(&String, &Vec<TypeImplLocation>)> = buckets.iter().collect();
    sorted.sort_by_key(|(name, _)| name.as_str());
    for (name, locations) in sorted {
        if let Some(v) = build_one(name, locations) {
            out.push(v);
        }
    }
    out
}

fn build_one(name: &str, locations: &[TypeImplLocation]) -> Option<Violation> {
    let count = locations.len() as u32;
    let (severity, threshold) = super::severity_for(
        count,
        TRAIT_IMPL_FANOUT_WARNING,
        TRAIT_IMPL_FANOUT_ERROR,
    )?;
    // Anchor the violation at the first impl site so the AI report
    // points the agent at a real line.
    let first = locations.first().expect("non-empty buckets only emit");
    let scope = name.to_string();
    let id = violation_id(&first.file, &scope, "trait-impl-fanout");
    Some(Violation {
        id,
        file: first.file.clone(),
        line: first.line,
        scope,
        scope_kind: ScopeKind::ImplBlock,
        metric: "trait-impl-fanout".into(),
        value: f64::from(count),
        threshold: f64::from(threshold),
        severity,
        rationale: Some(rationale_for(name, count, locations)),
        refactor_hints: REFACTOR_HINTS.iter().map(|s| s.to_string()).collect(),
        references: REFERENCES.iter().map(|s| s.to_string()).collect(),
        rust_context: Default::default(),
        complexity_justified: None,
    })
}

/// Per-type measurement: the number of impl blocks targeting each
/// type that appeared anywhere in the workspace. Anchored at the
/// first impl site so the report's `(file, scope)` join lands at
/// a real source location.
fn emit_measurements(
    buckets: &HashMap<String, Vec<TypeImplLocation>>,
) -> Vec<MeasurementRecord> {
    let mut out = Vec::with_capacity(buckets.len());
    let mut sorted: Vec<(&String, &Vec<TypeImplLocation>)> = buckets.iter().collect();
    sorted.sort_by_key(|(name, _)| name.as_str());
    for (name, locations) in sorted {
        let Some(first) = locations.first() else {
            continue;
        };
        out.push(MeasurementRecord {
            file: first.file.clone(),
            scope: name.clone(),
            metric: "trait-impl-fanout".into(),
            value: locations.len() as f64,
        });
    }
    out
}

fn rationale_for(name: &str, count: u32, locations: &[TypeImplLocation]) -> String {
    let mut s = format!(
        "`{name}` has {count} impl blocks targeting it. Many distinct impls \
         on one type often signal that the type is doing several jobs at once.\n\n\
         Sites:"
    );
    for loc in locations {
        s.push_str(&format!("\n  - {}:{}", loc.file, loc.line));
    }
    s
}

const REFACTOR_HINTS: &[&str] = &[
    "If the impls split cleanly by role (serde / display / domain logic), \
extract the marginal ones into a wrapper type and impl on that.",
    "Trait impls that only forward to one method are good candidates to \
move to a `*Ext` blanket.",
    "Multiple inherent impls (`impl Foo { ... }` repeated) can usually \
collapse into one block — splitting them is a stylistic choice and the \
fanout count exaggerates it.",
];

const REFERENCES: &[&str] = &[];

#[cfg(test)]
mod tests {
    static TEMPDIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    use super::*;
    use crate::discover::DiscoveredFile;
    use rustics::MetricSeverity;

    /// Parses a slice of `DiscoveredFile`s into `ParsedFile`s,
    /// dropping unreadable / unparseable entries — same contract
    /// as the production `super::parse_workspace_files`. Each test
    /// constructs a small `Vec<DiscoveredFile>`, then funnels
    /// through this so the test still exercises the read+parse path
    /// (just no longer inside `run`).
    fn parse_for_test(files: &[DiscoveredFile]) -> Vec<ParsedFile> {
        files
            .iter()
            .filter_map(|f| {
                let source = std::fs::read_to_string(&f.absolute).ok()?;
                let parsed = ra_ap_syntax::SourceFile::parse(
                    &source,
                    ra_ap_syntax::Edition::CURRENT,
                );
                Some(ParsedFile {
                    relative: f.relative.clone(),
                    tree: parsed.tree(),
                })
            })
            .collect()
    }

    fn parse_type(s: &str) -> ast::Type {
        let src = format!("type _X = {s};");
        let parsed = ra_ap_syntax::SourceFile::parse(&src, ra_ap_syntax::Edition::CURRENT);
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
    fn type_name_extracts_path_tail() {
        let ty = parse_type("crate::module::Foo");
        assert_eq!(type_name(&ty).as_deref(), Some("Foo"));
    }

    #[test]
    fn type_name_unwraps_reference() {
        let ty = parse_type("&'a Foo");
        assert_eq!(type_name(&ty).as_deref(), Some("Foo"));
    }

    #[test]
    fn type_name_unwraps_paren() {
        let ty = parse_type("(Foo)");
        assert_eq!(type_name(&ty).as_deref(), Some("Foo"));
    }

    #[test]
    fn type_name_returns_none_for_tuple() {
        let ty = parse_type("(u8, u16)");
        assert!(type_name(&ty).is_none());
    }

    fn write_file(dir: &std::path::Path, rel: &str, body: &str) -> DiscoveredFile {
        let abs = dir.join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, body).unwrap();
        DiscoveredFile {
            absolute: abs,
            relative: rel.to_string(),
        }
    }

    fn tempdir() -> std::path::PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = TEMPDIR_SEQ.fetch_add(
            1,
            std::sync::atomic::Ordering::Relaxed,
        );
        let path = std::env::temp_dir().join(format!("rustics-cross-test-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn trait_impl_fanout_emits_warning_for_heavy_type() {
        let tmp = tempdir();
        // Build 9 impls on `Heavy` across 3 files, plus 2 on `Light`.
        let body = (0..9)
            .map(|i| format!("impl Trait{i} for Heavy {{}}\n"))
            .collect::<String>();
        let files = vec![
            write_file(&tmp, "src/a.rs", &body),
            write_file(&tmp, "src/b.rs", "impl Foo for Light {}\nimpl Bar for Light {}\n"),
        ];
        let violations = run(&parse_for_test(&files)).violations;
        let heavy = violations.iter().find(|v| v.scope == "Heavy").expect("Heavy");
        assert_eq!(heavy.severity, MetricSeverity::Warning);
        assert_eq!(heavy.value, 9.0);
        assert_eq!(heavy.threshold, f64::from(TRAIT_IMPL_FANOUT_WARNING));
        // The first impl site anchors the violation.
        assert_eq!(heavy.file, "src/a.rs");
        // Light has only 2 impls — below threshold, no violation.
        assert!(violations.iter().all(|v| v.scope != "Light"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn trait_impl_fanout_emits_error_above_error_threshold() {
        let tmp = tempdir();
        let body = (0..18)
            .map(|i| format!("impl Trait{i} for Heavy {{}}\n"))
            .collect::<String>();
        let files = vec![write_file(&tmp, "src/a.rs", &body)];
        let violations = run(&parse_for_test(&files)).violations;
        let heavy = violations.iter().find(|v| v.scope == "Heavy").expect("Heavy");
        assert_eq!(heavy.severity, MetricSeverity::Error);
        assert_eq!(heavy.threshold, f64::from(TRAIT_IMPL_FANOUT_ERROR));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn trait_impl_fanout_skips_unreadable_or_unparseable_files() {
        let tmp = tempdir();
        // Real file with one impl, plus a missing-path entry, plus a
        // non-Rust file. Only the real file should contribute.
        let _ok = write_file(&tmp, "src/a.rs", "impl Foo for Heavy {}\n");
        let files = vec![
            DiscoveredFile {
                absolute: tmp.join("src/missing.rs"),
                relative: "src/missing.rs".into(),
            },
            DiscoveredFile {
                absolute: tmp.join("src/a.rs"),
                relative: "src/a.rs".into(),
            },
            write_file(&tmp, "src/junk.rs", ":: this is not :: rust ::"),
        ];
        // 1 impl on Heavy → no violation; just verifying no panic.
        let violations = run(&parse_for_test(&files)).violations;
        assert!(violations.is_empty());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn run_emits_measurement_for_every_type_with_impls() {
        // Even when no fanout-violation fires, every type with at
        // least one impl gets a measurement so `regression`'s
        // cosmetic-detection sees sub-threshold drifts (e.g. 6 → 7
        // impls without crossing 8).
        let tmp = tempdir();
        let files = vec![
            write_file(
                &tmp,
                "src/a.rs",
                "impl Foo for Bar {}\nimpl Baz for Bar {}\nimpl Qux for Other {}\n",
            ),
        ];
        let pass = run(&parse_for_test(&files));
        assert!(pass.violations.is_empty(), "no type crosses 8 impls");
        let bar = pass
            .measurements
            .iter()
            .find(|m| m.scope == "Bar")
            .expect("Bar measurement");
        assert_eq!(bar.value, 2.0);
        assert_eq!(bar.metric, "trait-impl-fanout");
        let other = pass
            .measurements
            .iter()
            .find(|m| m.scope == "Other")
            .expect("Other measurement");
        assert_eq!(other.value, 1.0);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn rationale_lists_each_site() {
        let locations = vec![
            TypeImplLocation { file: "a.rs".into(), line: 1 },
            TypeImplLocation { file: "b.rs".into(), line: 7 },
        ];
        let s = rationale_for("Foo", 9, &locations);
        assert!(s.contains("`Foo` has 9 impl blocks"));
        assert!(s.contains("a.rs:1"));
        assert!(s.contains("b.rs:7"));
    }
}
