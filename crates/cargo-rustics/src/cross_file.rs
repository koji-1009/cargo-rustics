//! Cross-file aggregations.
//!
//! Plan §6.2 — `trait-impl-fanout` is the count of impl blocks
//! targeting a single struct/enum across the whole workspace. The
//! per-file lens infrastructure (which underlies every M1 lens) does
//! not see other files; this module fills that gap by re-walking the
//! discovered file set and aggregating impl receivers.
//!
//! At M2 only `trait-impl-fanout` lives here. Other plan §6.3 cross-
//! file metrics (Afferent Coupling Ca, Instability I, Distance D)
//! land alongside the regression command's two-snapshot loader.

use std::collections::HashMap;

use rustics::{violation_id, MetricSeverity, ScopeKind};
use syn::{visit::Visit, ItemImpl, Type};

use crate::discover::DiscoveredFile;
use crate::report::Violation;

/// Threshold defaults — chosen by the same eye that picked the
/// per-impl-block ones. Plan §6.2.
const TRAIT_IMPL_FANOUT_WARNING: u32 = 8;
const TRAIT_IMPL_FANOUT_ERROR: u32 = 16;

/// Walks every discovered file's AST and emits one
/// `trait-impl-fanout` violation per type whose impl-block count
/// crosses the warning/error threshold.
pub fn trait_impl_fanout(files: &[DiscoveredFile]) -> Vec<Violation> {
    let mut buckets: HashMap<String, Vec<TypeImplLocation>> = HashMap::new();
    for file in files {
        collect_impls_in_file(file, &mut buckets);
    }
    emit_violations(&buckets)
}

#[derive(Debug, Clone)]
struct TypeImplLocation {
    file: String,
    line: usize,
}

fn collect_impls_in_file(
    file: &DiscoveredFile,
    buckets: &mut HashMap<String, Vec<TypeImplLocation>>,
) {
    let Ok(source) = std::fs::read_to_string(&file.absolute) else {
        return;
    };
    let Ok(ast) = syn::parse_file(&source) else {
        return;
    };
    let mut v = ImplCollector {
        out: buckets,
        relative: file.relative.clone(),
    };
    v.visit_file(&ast);
}

struct ImplCollector<'a> {
    out: &'a mut HashMap<String, Vec<TypeImplLocation>>,
    relative: String,
}

impl<'a, 'ast> Visit<'ast> for ImplCollector<'a> {
    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        if let Some(name) = type_name(&node.self_ty) {
            self.out.entry(name).or_default().push(TypeImplLocation {
                file: self.relative.clone(),
                line: node.impl_token.span.start().line,
            });
        }
    }
}

fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(tp) => tp.path.segments.last().map(|s| s.ident.to_string()),
        Type::Reference(r) => type_name(&r.elem),
        Type::Paren(p) => type_name(&p.elem),
        Type::Group(g) => type_name(&g.elem),
        _ => None,
    }
}

fn emit_violations(buckets: &HashMap<String, Vec<TypeImplLocation>>) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut sorted_types: Vec<(&String, &Vec<TypeImplLocation>)> = buckets.iter().collect();
    sorted_types.sort_by_key(|(name, _)| name.as_str());
    for (name, locations) in sorted_types {
        let count = locations.len() as u32;
        let Some((severity, threshold)) = severity_for(count) else {
            continue;
        };
        // Anchor the violation at the first impl site so the AI report
        // points the agent at a real line.
        let first = locations.first().expect("non-empty buckets only emit");
        let scope = name.clone();
        let id = violation_id(&first.file, &scope, "trait-impl-fanout");
        out.push(Violation {
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
        });
    }
    out
}

fn severity_for(count: u32) -> Option<(MetricSeverity, u32)> {
    if count > TRAIT_IMPL_FANOUT_ERROR {
        Some((MetricSeverity::Error, TRAIT_IMPL_FANOUT_ERROR))
    } else if count > TRAIT_IMPL_FANOUT_WARNING {
        Some((MetricSeverity::Warning, TRAIT_IMPL_FANOUT_WARNING))
    } else {
        None
    }
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

const REFERENCES: &[&str] = &["plan §6.2 — trait-impl-fanout (cross-file)."];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_below_warning_is_none() {
        assert!(severity_for(5).is_none());
    }

    #[test]
    fn severity_at_warning_threshold_is_none() {
        assert!(severity_for(8).is_none());
    }

    #[test]
    fn severity_above_warning_is_warning() {
        let (s, t) = severity_for(9).unwrap();
        assert_eq!(s, MetricSeverity::Warning);
        assert_eq!(t, TRAIT_IMPL_FANOUT_WARNING);
    }

    #[test]
    fn severity_above_error_is_error() {
        let (s, t) = severity_for(20).unwrap();
        assert_eq!(s, MetricSeverity::Error);
        assert_eq!(t, TRAIT_IMPL_FANOUT_ERROR);
    }

    #[test]
    fn type_name_extracts_path_tail() {
        let ty: Type = syn::parse_str("crate::module::Foo").unwrap();
        assert_eq!(type_name(&ty).as_deref(), Some("Foo"));
    }

    #[test]
    fn type_name_unwraps_reference() {
        let ty: Type = syn::parse_str("&'a Foo").unwrap();
        assert_eq!(type_name(&ty).as_deref(), Some("Foo"));
    }

    #[test]
    fn type_name_unwraps_paren_and_group() {
        let ty: Type = syn::parse_str("(Foo)").unwrap();
        assert_eq!(type_name(&ty).as_deref(), Some("Foo"));
    }

    #[test]
    fn type_name_returns_none_for_tuple() {
        let ty: Type = syn::parse_str("(u8, u16)").unwrap();
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
        let path = std::env::temp_dir().join(format!("rustics-cross-test-{pid}-{n}"));
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
        let violations = trait_impl_fanout(&files);
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
        let violations = trait_impl_fanout(&files);
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
        let violations = trait_impl_fanout(&files);
        assert!(violations.is_empty());
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
