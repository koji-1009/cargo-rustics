//! HIR-based unused detector — Layer 2 backend.
//!
//! Walks every workspace-local `Definition` reachable through HIR,
//! looks up its references via `ra_ap_ide_db::search`, and emits an
//! [`UnusedItem`] for every public definition whose reference set is
//! empty. Unlike the `ra_ap_syntax`-only detector this disambiguates
//! homonyms across modules and resolves method-dispatch through
//! macro bodies — the two Layer 1 false-positive classes that
//! motivate this backend.

use anyhow::Result;
use std::path::Path;

use ra_ap_hir::{attach_db, AssocItem, Crate, HasVisibility, ModuleDef, Semantics, Visibility};
use ra_ap_ide::TryToNav;
use ra_ap_ide_db::{defs::Definition, search::ReferenceCategory, RootDatabase};

/// One HIR-resolved unused finding.
///
/// Field set deliberately mirrors `cargo_rustics::unused::UnusedItem`
/// (`kind` as `&'static str` rather than `String` because every
/// classify_* path produces a static; the cargo-rustics wiring layer
/// does the conversion to the `String`-keyed report shape).
#[derive(Debug, Clone)]
pub struct UnusedItem {
    pub file: String,
    pub line: u32,
    pub name: String,
    pub kind: &'static str,
    /// Containing type for inherent-impl methods / associated consts.
    /// `None` for module-level items, matching the Layer 1 detector.
    pub parent: Option<String>,
}

/// Loads `manifest_dir` and returns every `pub` definition with
/// zero references outside the declaration site. Roots:
/// `fn main`, `#[test]`, `#[no_mangle]`, `#[export_name]` — handled
/// implicitly because `ide_db::search` treats them as having
/// well-known callers.
pub fn detect_at(manifest_dir: &Path) -> Result<Vec<UnusedItem>> {
    let workspace = crate::workspace::load(manifest_dir)?;
    let db = workspace.host.raw_database();
    // ra_ap_hir_ty's next-gen trait solver caches the active db in
    // a thread-local; queries that touch the solver panic with
    // "Try to use attached db, but no db is attached" if we don't
    // open a TLS scope first. `attach_db` does that for the closure.
    let host = &workspace.host;
    let vfs = &workspace.vfs;
    attach_db(db, || -> Result<Vec<UnusedItem>> {
        let mut out = Vec::new();
        for krate in Crate::all(db) {
            // Skip stdlib / proc_macro / external libraries — for an
            // "unused" report we only care about workspace-member
            // crates the user can actually edit. `CrateOrigin::Local`
            // is the workspace-member tag.
            if !krate.origin(db).is_local() {
                continue;
            }
            collect_unused_in_crate(db, krate, host, vfs, &mut out)?;
        }
        Ok(out)
    })
}

fn collect_unused_in_crate(
    db: &RootDatabase,
    krate: Crate,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
    out: &mut Vec<UnusedItem>,
) -> Result<()> {
    let root_module = krate.root_module(db);
    let mut stack = vec![root_module];
    while let Some(module) = stack.pop() {
        for child in module.children(db) {
            stack.push(child);
        }
        collect_unused_in_module(db, module, host, vfs, out)?;
    }
    Ok(())
}

/// Per-module walk: top-level declarations + inherent-impl items.
/// Pulled out of [`collect_unused_in_crate`] so each function stays
/// under cognitive-complexity / cyclomatic-complexity warnings —
/// the BFS over modules is one shape, the dispatch over the two
/// declaration kinds is another.
fn collect_unused_in_module(
    db: &RootDatabase,
    module: ra_ap_hir::Module,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
    out: &mut Vec<UnusedItem>,
) -> Result<()> {
    for decl in module.declarations(db) {
        if let Some(item) = check_module_def(db, decl, host, vfs)? {
            out.push(item);
        }
    }
    // Inherent impl items — methods, associated consts. Trait impls
    // are skipped: their method set is dictated by the trait
    // contract, not a cohesion choice on the type. The matching
    // Layer 1 detector applies the same skip rule.
    for impl_def in module.impl_defs(db) {
        if impl_def.trait_(db).is_some() {
            continue;
        }
        let parent = impl_self_ty_name(db, impl_def);
        for assoc in impl_def.items(db) {
            if let Some(item) = check_assoc_item(db, assoc, parent.as_deref(), host, vfs)? {
                out.push(item);
            }
        }
    }
    Ok(())
}

/// `AssocItem` parallel of [`check_module_def`]. Inherent-impl
/// methods / associated consts are the second class of declarations
/// the Layer 1 detector surfaces; HIR's resolution is what lets us
/// disambiguate them through macro bodies (the `eprintln!("{}",
/// c.method())` case the unexpanded AST cannot see).
fn check_assoc_item(
    db: &RootDatabase,
    assoc: AssocItem,
    parent: Option<&str>,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
) -> Result<Option<UnusedItem>> {
    if !assoc_item_is_public(db, assoc) {
        return Ok(None);
    }
    let Some((definition, kind)) = classify_assoc(assoc) else {
        return Ok(None);
    };
    let sema = Semantics::new(db);
    if has_live_reference(&definition, &sema) {
        return Ok(None);
    }
    let Some(nav) = definition.try_to_nav(&sema) else {
        return Ok(None);
    };
    build_unused_item(nav.call_site, kind, parent.map(str::to_string), host, vfs)
}

/// Resolves an `impl T { … }` block to the display name of `T`
/// when `T` is an ADT (struct / enum / union). Returns `None` for
/// impls on tuples, references, or other non-ADT types — those
/// don't have a stable single-word "containing type" to report
/// alongside an unused method.
fn impl_self_ty_name(db: &RootDatabase, impl_def: ra_ap_hir::Impl) -> Option<String> {
    let adt = impl_def.self_ty(db).as_adt()?;
    Some(adt.name(db).as_str().to_owned())
}

/// Maps an inherent-impl associated item to `(Definition, kind)`.
/// `AssocItem::TypeAlias` is intentionally dropped — Layer 1
/// surfaces methods and associated consts only, so the two
/// detectors keep the same output shape.
fn classify_assoc(assoc: AssocItem) -> Option<(Definition, &'static str)> {
    match assoc {
        AssocItem::Function(f) => Some((Definition::Function(f), "method")),
        AssocItem::Const(c) => Some((Definition::Const(c), "assoc-const")),
        AssocItem::TypeAlias(_) => None,
    }
}

fn check_module_def(
    db: &RootDatabase,
    decl: ModuleDef,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
) -> Result<Option<UnusedItem>> {
    if !module_def_is_public(db, decl) {
        return Ok(None);
    }
    let Some((definition, kind)) = classify(decl) else {
        return Ok(None);
    };
    let sema = Semantics::new(db);
    if has_live_reference(&definition, &sema) {
        return Ok(None);
    }
    let Some(nav) = definition.try_to_nav(&sema) else {
        return Ok(None);
    };
    build_unused_item(nav.call_site, kind, None, host, vfs)
}

/// True iff `decl` has any `pub` visibility annotation
/// (`pub`, `pub(crate)`, `pub(super)`, `pub(in path)`). Matches the
/// Layer 1 detector's "surface every item declared with a `pub`
/// keyword" semantics — strictly-private items (`fn foo()` with no
/// modifier) are not part of the public-API report.
fn module_def_is_public(db: &RootDatabase, decl: ModuleDef) -> bool {
    let vis = match decl {
        ModuleDef::Function(f) => f.visibility(db),
        ModuleDef::Adt(a) => a.visibility(db),
        ModuleDef::Trait(t) => t.visibility(db),
        ModuleDef::TypeAlias(t) => t.visibility(db),
        ModuleDef::Const(c) => c.visibility(db),
        ModuleDef::Static(s) => s.visibility(db),
        _ => return false,
    };
    !matches!(vis, Visibility::Module(_, _))
}

fn assoc_item_is_public(db: &RootDatabase, assoc: AssocItem) -> bool {
    let vis = match assoc {
        AssocItem::Function(f) => f.visibility(db),
        AssocItem::Const(c) => c.visibility(db),
        AssocItem::TypeAlias(t) => t.visibility(db),
    };
    !matches!(vis, Visibility::Module(_, _))
}

/// Maps a `ModuleDef` we want to surface to `(Definition, kind)`.
/// Returns `None` for kinds the unused detector ignores
/// (modules / macros / external blocks / etc).
fn classify(decl: ModuleDef) -> Option<(Definition, &'static str)> {
    match decl {
        ModuleDef::Function(f) => Some((Definition::Function(f), "fn")),
        ModuleDef::Adt(a) => Some((Definition::Adt(a), "adt")),
        ModuleDef::Trait(t) => Some((Definition::Trait(t), "trait")),
        ModuleDef::TypeAlias(t) => Some((Definition::TypeAlias(t), "type")),
        ModuleDef::Const(c) => Some((Definition::Const(c), "const")),
        ModuleDef::Static(s) => Some((Definition::Static(s), "static")),
        _ => None,
    }
}

/// `true` when the definition has any reference that isn't an
/// `IMPORT`-flagged occurrence — `pub use` re-exports alone are not
/// considered "use".
///
/// No `.in_scope()` restriction: the default search reaches every
/// file the workspace knows about. Per-crate `module_and_children`
/// scope was the original spike's choice, but it failed the cross-
/// crate consumer case — `ai_report_contract_version` defined in
/// the `rustics` lib and consumed from `cargo-rustics` looked
/// unused under that scope. Leaving the scope unbounded is correct;
/// search is local-crate-only via `CrateOrigin::Local` filtering at
/// the definition-iteration side.
fn has_live_reference(definition: &Definition, sema: &Semantics<'_, RootDatabase>) -> bool {
    let usages = definition.usages(sema).all();
    usages.references.values().any(|refs| {
        refs.iter()
            .any(|r| !r.category.contains(ReferenceCategory::IMPORT))
    })
}

/// Builds the `UnusedItem` payload from a navigation target — file,
/// line, name. Pulled out so the locator path and the rendering path
/// don't share a single big function.
fn build_unused_item(
    target: ra_ap_ide::NavigationTarget,
    kind: &'static str,
    parent: Option<String>,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
) -> Result<Option<UnusedItem>> {
    let name = target.name.to_string();
    // Anonymous items (`const _: () = …;` blocks emitted by derive
    // macros, blanket trait-impl assertions, etc.) carry no surface
    // identifier the user can act on. Layer 1 misses them because
    // they're inside macro bodies; Layer 2 sees them post-expansion
    // and they show up as `_`-named entries with no useful signal.
    // Drop them at the report boundary.
    if name == "_" {
        return Ok(None);
    }
    let analysis = host.analysis();
    let line_index = analysis.file_line_index(target.file_id)?;
    let line = line_index
        .line_col(target.focus_or_full_range().start())
        .line;
    let path = vfs
        .file_path(target.file_id)
        .as_path()
        .map(|p| p.to_string())
        .unwrap_or_default();
    Ok(Some(UnusedItem {
        file: path,
        line,
        name,
        kind,
        parent,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMPDIR_SEQ: AtomicU64 = AtomicU64::new(0);

    /// Workspace fixture exercising the two Layer-1 blind spots the
    /// HIR detector is meant to fix:
    ///
    /// 1. **Homonym disambiguation.** `mod_a::helper` and
    ///    `mod_b::helper` are two distinct items with the same name.
    ///    Only `mod_a::helper` is called. The syn token-counting
    ///    detector credits the single `helper` call to both and
    ///    flags neither; HIR resolves `mod_a::helper` exactly and
    ///    flags `mod_b::helper` correctly.
    /// 2. **Method call inside a macro body.** `Calc::method_in_macro`
    ///    is invoked only through `eprintln!("{}", c.method_in_macro())`.
    ///    `ra_ap_syntax` does not walk macro contents at parse time
    ///    so the call is invisible to the Layer 1 walker. HIR sees
    ///    post-expansion code and resolves the receiver.
    const FIXTURE_CARGO_TOML: &str = "[package]
name = \"fixture\"
version = \"0.1.0\"
edition = \"2021\"
publish = false

[lib]
path = \"src/lib.rs\"
";

    const FIXTURE_LIB_RS: &str = "pub fn entry() {
    mod_a::helper();
    let c = Calc { x: 1 };
    // Positional macro-arg form.
    eprintln!(\"{}\", c.method_in_macro());
    // Named-format-arg form with a conditional. This mirrors the
    // real call shape in cargo-rustics::statistics::print_to_stderr
    // that surfaced is_redundant as a false positive.
    eprintln!(
        \"flag={f}\",
        f = if c.method_in_named_arg() { \"x\" } else { \"y\" },
    );
}

pub mod mod_a {
    pub fn helper() {}
}

pub mod mod_b {
    // Same name as mod_a::helper; never called. The Layer 1 detector
    // misses this; HIR catches it.
    pub fn helper() {}
}

pub struct Calc {
    pub x: i32,
}

impl Calc {
    // Called only inside an eprintln! body. Layer 1 misses; HIR
    // resolves it.
    pub fn method_in_macro(&self) -> i32 {
        self.x
    }

    // Called only inside the named-format-arg form of eprintln!.
    // This is the shape that triggered the is_redundant false
    // positive in the real workspace, so the test must prove HIR
    // resolves through it.
    pub fn method_in_named_arg(&self) -> bool {
        true
    }

    // Genuinely unused — a control that detects whether the
    // detector classifies impl methods at all. If `never_method`
    // is not flagged, classify() is silently skipping every impl
    // item and the other negative assertions below would be passing
    // for the wrong reason.
    pub fn never_method(&self) -> i32 {
        0
    }
}

pub fn never_called() {}
";

    fn unique_workspace(label: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = TEMPDIR_SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("rustics-ra-{label}-{pid}-{n}-{seq}"))
    }

    fn write_fixture() -> std::path::PathBuf {
        let dir = unique_workspace("unused");
        fs::create_dir_all(dir.join("src")).expect("create src/");
        fs::write(dir.join("Cargo.toml"), FIXTURE_CARGO_TOML).expect("write Cargo.toml");
        fs::write(dir.join("src/lib.rs"), FIXTURE_LIB_RS).expect("write lib.rs");
        dir
    }

    #[test]
    fn hir_detector_catches_homonym_and_method_in_macro() {
        let dir = write_fixture();
        let items = detect_at(&dir).expect("detect_at");
        let names: std::collections::BTreeSet<&str> =
            items.iter().map(|i| i.name.as_str()).collect();

        // The Layer 1 detector flags neither of these — that's the
        // bug. The HIR detector must flag both.
        //
        // We can't pin "exactly which `helper`": both have the same
        // name. The contract is "helper appears in the unused list
        // because at least one of the two is dead" — and on this
        // fixture, only mod_b::helper is dead, so the count of
        // `helper` entries is exactly 1.
        let helper_count = items.iter().filter(|i| i.name == "helper").count();
        assert_eq!(
            helper_count, 1,
            "exactly one of mod_a::helper / mod_b::helper should be unused (mod_b's), \
             got names: {names:?}"
        );

        assert!(
            names.contains("never_called"),
            "never_called must be flagged unused, got: {names:?}"
        );

        // Control assertion — `never_method` is an inherent-impl
        // method that nothing references. If the detector classifies
        // impl items at all, it MUST surface this. If this fails,
        // the macro-resolution assertion below would be passing for
        // the wrong reason (impl items silently dropped before any
        // reference search).
        let surfaces_impl_methods = names.contains("never_method");

        // Negative assertions (only meaningful if impl methods are
        // actually classified) — both methods are referenced from
        // inside an eprintln! body and HIR must see through.
        if surfaces_impl_methods {
            assert!(
                !names.contains("method_in_macro"),
                "method_in_macro is called inside eprintln!() and HIR must \
                 resolve through the macro expansion; got: {names:?}"
            );
            assert!(
                !names.contains("method_in_named_arg"),
                "method_in_named_arg is called inside the named-format-arg \
                 form of eprintln!() (the real-workspace shape that surfaced \
                 is_redundant); HIR must resolve through it; got: {names:?}"
            );
        } else {
            // Make the gap explicit so a future change that adds
            // impl-item walking trips the assertion and forces the
            // macro-resolution branch to start firing.
            panic!(
                "HIR detector does not classify impl methods; \
                 method_in_macro / never_method both invisible. \
                 Extend classify() to walk inherent impl items \
                 before asserting macro-resolution. Names seen: {names:?}"
            );
        }

        // Cleanup. Leave on assertion failure so the user can inspect.
        let _ = fs::remove_dir_all(&dir);
    }
}
