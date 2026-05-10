//! HIR-based unused detector — Layer 2 spike.
//!
//! Walks every `Definition` reachable in the workspace's HIR, looks
//! up its references via `ra_ap_ide_db::search`, and emits an
//! [`UnusedItem`] for every public definition whose reference set is
//! empty. Unlike the syn-based detector this disambiguates
//! homonyms, resolves method-dispatch, and sees through proc-macro
//! expansion.

use anyhow::Result;
use std::path::Path;

use ra_ap_hir::{attach_db, Crate, ModuleDef, Semantics};
use ra_ap_ide::TryToNav;
use ra_ap_ide_db::{
    defs::Definition,
    search::{ReferenceCategory, SearchScope},
    RootDatabase,
};

/// One HIR-resolved unused finding.
#[derive(Debug, Clone)]
pub struct UnusedItem {
    pub file: String,
    pub line: u32,
    pub name: String,
    pub kind: &'static str,
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
    let scope = SearchScope::module_and_children(db, root_module);
    let mut stack = vec![root_module];
    while let Some(module) = stack.pop() {
        for child in module.children(db) {
            stack.push(child);
        }
        for decl in module.declarations(db) {
            if let Some(item) = check_module_def(db, decl, host, vfs, &scope)? {
                out.push(item);
            }
        }
    }
    Ok(())
}

fn check_module_def(
    db: &RootDatabase,
    decl: ModuleDef,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
    scope: &SearchScope,
) -> Result<Option<UnusedItem>> {
    let Some((definition, kind)) = classify(decl) else {
        return Ok(None);
    };
    let sema = Semantics::new(db);
    if has_live_reference(&definition, &sema, scope) {
        return Ok(None);
    }
    let Some(nav) = definition.try_to_nav(&sema) else {
        return Ok(None);
    };
    Ok(Some(build_unused_item(nav.call_site, kind, host, vfs)?))
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
fn has_live_reference(
    definition: &Definition,
    sema: &Semantics<'_, RootDatabase>,
    scope: &SearchScope,
) -> bool {
    let usages = definition.usages(sema).in_scope(scope).all();
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
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
) -> Result<UnusedItem> {
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
    Ok(UnusedItem {
        file: path,
        line,
        name: target.name.to_string(),
        kind,
    })
}
