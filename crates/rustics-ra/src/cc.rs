//! HIR-backed cyclomatic complexity — Layer 2 spike.
//!
//! Mirrors the syn-based `cyclomatic-complexity` lens
//! (`crates/rustics/src/metrics/cyclomatic_complexity.rs`) but
//! sources its AST from `ra_ap_syntax` instead of `syn`. The
//! decision-point set is identical:
//!
//! * +1 per `if` / `else if` / `if let`
//! * +1 per `while` / `while let`
//! * +1 per `for`
//! * +1 per `loop`
//! * +N-1 per `match` arm count *only* when a `_` wildcard arm
//!   exists; sealed (no-wildcard) match contributes 0
//! * +1 per `&&` / `||`
//! * +1 per `?`
//! * baseline 1
//!
//! Crate filter is `CrateOrigin::Local`; only workspace-member
//! functions get measurements emitted.
//!
//! This module exists to answer one question for the spike: do the
//! HIR-walked numbers match the syn-walked numbers function-for-
//! function? If they do, migrating CC from syn to HIR is safe; if
//! they don't, the divergence is interesting data either way (HIR
//! exposes macro-expanded code that `syn::Visit` skips, so
//! mismatch would *typically* mean HIR catches branches inside a
//! `vec![...]` that syn doesn't).

use anyhow::Result;
use std::path::Path;

use ra_ap_hir::{attach_db, AssocItem, Crate, HasSource, Impl, Module, ModuleDef};
use ra_ap_syntax::{
    ast::{self, AstNode, BinaryOp, HasName, LogicOp, Pat},
    SyntaxKind, SyntaxNode,
};

/// One per-function CC measurement.
#[derive(Debug, Clone)]
pub struct CcRow {
    pub file: String,
    pub line: u32,
    pub scope: String,
    pub cc: u32,
}

/// Top-level entry point: open the workspace with default options,
/// walk every workspace-member crate, compute CC per function,
/// return rows.
pub fn measure_at(manifest_dir: &Path) -> Result<Vec<CcRow>> {
    measure_at_with(manifest_dir, crate::workspace::LoadOpts::default())
}

/// Same as [`measure_at`] but with explicit load options. Used by
/// the `load_bench` example to time alternative load
/// configurations end-to-end.
pub fn measure_at_with(
    manifest_dir: &Path,
    opts: crate::workspace::LoadOpts,
) -> Result<Vec<CcRow>> {
    let workspace = crate::workspace::load_with(manifest_dir, opts)?;
    measure_loaded(&workspace)
}

/// HIR-bypass alternative: walk every workspace-local `.rs` file
/// via `ra_ap_syntax`'s parser directly, skipping the HIR query
/// pipeline. Produces equivalent CC numbers because CC is a
/// syntax-shape metric. Useful as a calibration baseline against
/// [`measure_loaded`] to see how much overhead the HIR queries
/// add.
pub fn measure_loaded_syntax(workspace: &crate::workspace::LoadedWorkspace) -> Result<Vec<CcRow>> {
    use ra_ap_syntax::SourceFile;
    let db = workspace.host.raw_database();
    let analysis = workspace.host.analysis();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    attach_db(db, || -> Result<()> {
        for krate in Crate::all(db) {
            if !krate.origin(db).is_local() {
                continue;
            }
            // Get the crate's root_file plus every file referenced
            // from a module declaration. We walk modules to get
            // their file ids via the source map.
            let root_file = krate.root_file(db);
            collect_file(
                db,
                workspace,
                root_file,
                &analysis,
                &mut seen,
                &mut out,
            )?;
            // Walk every Module of the crate and collect its file
            // id too. `Crate::modules` covers every module
            // including submodules in their own files.
            for module in krate.modules(db) {
                let hir_file_id = module.definition_source_file_id(db);
                let real_file = match hir_file_id {
                    ra_ap_hir::HirFileId::FileId(f) => f.file_id(db),
                    ra_ap_hir::HirFileId::MacroFile(_) => continue,
                };
                collect_file(
                    db,
                    workspace,
                    real_file,
                    &analysis,
                    &mut seen,
                    &mut out,
                )?;
            }
            let _ = (root_file, SourceFile::parse); // keep imports referenced
        }
        Ok(())
    })?;
    Ok(out)
}

/// Reads `file_id` from the loaded VFS, parses it with
/// `ra_ap_syntax`, walks the AST for CC. Skips files we've seen
/// already (so multi-module crates don't double-count the root).
fn collect_file(
    _db: &ra_ap_ide_db::RootDatabase,
    workspace: &crate::workspace::LoadedWorkspace,
    file_id: ra_ap_vfs::FileId,
    analysis: &ra_ap_ide::Analysis,
    seen: &mut std::collections::HashSet<ra_ap_vfs::FileId>,
    out: &mut Vec<CcRow>,
) -> Result<()> {
    if !seen.insert(file_id) {
        return Ok(());
    }
    let path = workspace
        .vfs
        .file_path(file_id)
        .as_path()
        .map(|p| p.to_string())
        .unwrap_or_default();
    let text = analysis.file_text(file_id)?;
    let parsed = ra_ap_syntax::SourceFile::parse(&text, ra_ap_syntax::Edition::CURRENT);
    let line_index = analysis.file_line_index(file_id)?;
    walk_syntax_for_cc(parsed.tree().syntax(), &path, &line_index, out);
    Ok(())
}

fn walk_syntax_for_cc(
    root: &ra_ap_syntax::SyntaxNode,
    file: &str,
    line_index: &ra_ap_ide::LineIndex,
    out: &mut Vec<CcRow>,
) {
    let mut scope_chain: Vec<String> = Vec::new();
    walk_syntax_scope(root, file, line_index, &mut scope_chain, out);
}

fn walk_syntax_scope(
    node: &ra_ap_syntax::SyntaxNode,
    file: &str,
    line_index: &ra_ap_ide::LineIndex,
    scope_chain: &mut Vec<String>,
    out: &mut Vec<CcRow>,
) {
    use ra_ap_syntax::AstNode;
    for child in node.children() {
        if try_record_fn(&child, file, line_index, scope_chain, out) {
            continue;
        }
        if let Some(m) = ast::Module::cast(child.clone()) {
            walk_module_scope(&m, file, line_index, scope_chain, out);
            continue;
        }
        if let Some(impl_node) = ast::Impl::cast(child.clone()) {
            walk_impl_scope(&impl_node, file, line_index, scope_chain, out);
            continue;
        }
        if let Some(trait_node) = ast::Trait::cast(child.clone()) {
            walk_trait_scope(&trait_node, file, line_index, scope_chain, out);
            continue;
        }
        walk_syntax_scope(&child, file, line_index, scope_chain, out);
    }
}

fn try_record_fn(
    child: &ra_ap_syntax::SyntaxNode,
    file: &str,
    line_index: &ra_ap_ide::LineIndex,
    scope_chain: &[String],
    out: &mut Vec<CcRow>,
) -> bool {
    use ra_ap_syntax::{ast::HasName, AstNode};
    let Some(fn_node) = ast::Fn::cast(child.clone()) else {
        return false;
    };
    let Some(name) = fn_node.name() else {
        return true;
    };
    let cc = count_cc(fn_node.syntax());
    let line = line_index.line_col(fn_node.syntax().text_range().start()).line;
    let mut scope = scope_chain.join("::");
    if !scope.is_empty() {
        scope.push_str("::");
    }
    scope.push_str(&name.text());
    out.push(CcRow {
        file: file.to_string(),
        line,
        scope,
        cc,
    });
    true
}

fn walk_module_scope(
    m: &ast::Module,
    file: &str,
    line_index: &ra_ap_ide::LineIndex,
    scope_chain: &mut Vec<String>,
    out: &mut Vec<CcRow>,
) {
    use ra_ap_syntax::{ast::HasName, AstNode};
    let pushed = m.name().map(|n| {
        scope_chain.push(n.text().to_string());
    });
    if let Some(item_list) = m.item_list() {
        walk_syntax_scope(item_list.syntax(), file, line_index, scope_chain, out);
    }
    if pushed.is_some() {
        scope_chain.pop();
    }
}

fn walk_impl_scope(
    impl_node: &ast::Impl,
    file: &str,
    line_index: &ra_ap_ide::LineIndex,
    scope_chain: &mut Vec<String>,
    out: &mut Vec<CcRow>,
) {
    use ra_ap_syntax::AstNode;
    let parent_name = impl_self_name(impl_node);
    scope_chain.push(parent_name);
    if let Some(assoc_list) = impl_node.assoc_item_list() {
        walk_syntax_scope(assoc_list.syntax(), file, line_index, scope_chain, out);
    }
    scope_chain.pop();
}

fn impl_self_name(impl_node: &ast::Impl) -> String {
    impl_node
        .self_ty()
        .as_ref()
        .and_then(|ty| match ty {
            ast::Type::PathType(p) => p.path().and_then(|p| p.segment()).map(|s| s.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

fn walk_trait_scope(
    trait_node: &ast::Trait,
    file: &str,
    line_index: &ra_ap_ide::LineIndex,
    scope_chain: &mut Vec<String>,
    out: &mut Vec<CcRow>,
) {
    use ra_ap_syntax::{ast::HasName, AstNode};
    let pushed = trait_node.name().map(|n| {
        scope_chain.push(n.text().to_string());
    });
    if let Some(assoc_list) = trait_node.assoc_item_list() {
        walk_syntax_scope(assoc_list.syntax(), file, line_index, scope_chain, out);
    }
    if pushed.is_some() {
        scope_chain.pop();
    }
}

/// Same as [`measure_at`] but against an already-loaded workspace.
/// Lets callers amortise the load cost across multiple lens passes.
pub fn measure_loaded(workspace: &crate::workspace::LoadedWorkspace) -> Result<Vec<CcRow>> {
    let db = workspace.host.raw_database();
    let host = &workspace.host;
    let vfs = &workspace.vfs;
    attach_db(db, || -> Result<Vec<CcRow>> {
        let mut out = Vec::new();
        for krate in Crate::all(db) {
            if !krate.origin(db).is_local() {
                continue;
            }
            // Bucket every inherent impl in the crate by its
            // self-type Adt once. Without this, `adt_inherent_fns`
            // re-iterates every impl per Adt — O(N×M) on
            // workspaces with many types.
            let impl_index = build_impl_index(db, krate);
            let root = krate.root_module(db);
            walk_module(
                db,
                root,
                host,
                vfs,
                &mut out,
                &mut Vec::new(),
                &impl_index,
            )?;
        }
        Ok(out)
    })
}

/// Pre-bucketed `Impl` list keyed by Adt. Built once per crate.
type ImplIndex = std::collections::HashMap<ra_ap_hir::Adt, Vec<Impl>>;

fn build_impl_index(db: &ra_ap_ide_db::RootDatabase, krate: Crate) -> ImplIndex {
    let mut index: ImplIndex = std::collections::HashMap::new();
    for imp in Impl::all_in_crate(db, krate) {
        // Only inherent impls (no trait, no negative impl). For
        // trait impls we'd want to bucket by trait, but the CC
        // walker currently does not enumerate trait-impl methods.
        if imp.trait_(db).is_some() {
            continue;
        }
        if let Some(adt) = imp.self_ty(db).as_adt() {
            index.entry(adt).or_default().push(imp);
        }
    }
    index
}

fn walk_module(
    db: &ra_ap_ide_db::RootDatabase,
    module: Module,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
    out: &mut Vec<CcRow>,
    scope_chain: &mut Vec<String>,
    impl_index: &ImplIndex,
) -> Result<()> {
    let pushed = push_module_name(db, module, scope_chain);
    for decl in module.declarations(db) {
        process_decl(db, decl, host, vfs, out, scope_chain, impl_index)?;
    }
    for child in module.children(db) {
        walk_module(db, child, host, vfs, out, scope_chain, impl_index)?;
    }
    if pushed {
        scope_chain.pop();
    }
    Ok(())
}

fn push_module_name(
    db: &ra_ap_ide_db::RootDatabase,
    module: Module,
    scope_chain: &mut Vec<String>,
) -> bool {
    let Some(name) = module.name(db) else {
        return false;
    };
    scope_chain.push(name.as_str().to_string());
    true
}

fn process_decl(
    db: &ra_ap_ide_db::RootDatabase,
    decl: ModuleDef,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
    out: &mut Vec<CcRow>,
    scope_chain: &mut Vec<String>,
    impl_index: &ImplIndex,
) -> Result<()> {
    match decl {
        ModuleDef::Function(f) => {
            if let Some(row) = measure_function(db, f, host, vfs, scope_chain)? {
                out.push(row);
            }
        }
        ModuleDef::Adt(_) | ModuleDef::Trait(_) => {
            walk_associated_fns(db, decl, host, vfs, out, scope_chain, impl_index)?;
        }
        _ => {}
    }
    Ok(())
}

/// Visits associated functions on Adt / Trait decls so methods get
/// measured too. Mirrors the syn-side walker shape.
fn walk_associated_fns(
    db: &ra_ap_ide_db::RootDatabase,
    decl: ModuleDef,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
    out: &mut Vec<CcRow>,
    scope_chain: &mut Vec<String>,
    impl_index: &ImplIndex,
) -> Result<()> {
    let Some((parent_name, fns)) = collect_assoc_fns(db, decl, impl_index) else {
        return Ok(());
    };
    scope_chain.push(parent_name);
    for f in fns {
        if let Some(row) = measure_function(db, f, host, vfs, scope_chain)? {
            out.push(row);
        }
    }
    scope_chain.pop();
    Ok(())
}

/// For an Adt or Trait `ModuleDef`, returns its name and the list
/// of associated `Function`s the CC measurement should include.
/// Adt's inherent impls are looked up in the pre-built impl index
/// to avoid an O(N×M) per-Adt scan.
fn collect_assoc_fns(
    db: &ra_ap_ide_db::RootDatabase,
    decl: ModuleDef,
    impl_index: &ImplIndex,
) -> Option<(String, Vec<ra_ap_hir::Function>)> {
    match decl {
        ModuleDef::Adt(a) => Some((
            a.name(db).as_str().to_string(),
            adt_inherent_fns(db, a, impl_index),
        )),
        ModuleDef::Trait(t) => Some((t.name(db).as_str().to_string(), trait_fns(db, t))),
        _ => None,
    }
}

fn adt_inherent_fns(
    db: &ra_ap_ide_db::RootDatabase,
    a: ra_ap_hir::Adt,
    impl_index: &ImplIndex,
) -> Vec<ra_ap_hir::Function> {
    let mut fns = Vec::new();
    let Some(impls) = impl_index.get(&a) else {
        return fns;
    };
    for imp in impls {
        fns.extend(imp.items(db).into_iter().filter_map(|i| match i {
            AssocItem::Function(f) => Some(f),
            _ => None,
        }));
    }
    fns
}

fn trait_fns(db: &ra_ap_ide_db::RootDatabase, t: ra_ap_hir::Trait) -> Vec<ra_ap_hir::Function> {
    t.items(db)
        .into_iter()
        .filter_map(|i| match i {
            AssocItem::Function(f) => Some(f),
            _ => None,
        })
        .collect()
}

fn measure_function(
    db: &ra_ap_ide_db::RootDatabase,
    f: ra_ap_hir::Function,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
    scope_chain: &[String],
) -> Result<Option<CcRow>> {
    let Some(source) = f.source(db) else {
        return Ok(None);
    };
    // `f.source(db)` may return a macro-expanded `HirFileId` for
    // proc-macro / `#[derive]` -generated functions. The expanded
    // syntax tree's `text_range` is in macro-output coordinates,
    // not original-file coordinates, so we can't index it against
    // the original file's `line_index` (line-index panics with
    // "invalid offset"). Skip macro-defined functions for now;
    // matches the syn-side behaviour, which can't see them either.
    let real_file_id = match source.file_id {
        ra_ap_hir::HirFileId::FileId(real) => real,
        ra_ap_hir::HirFileId::MacroFile(_) => return Ok(None),
    };
    let ast_fn = source.value;
    let Some(name) = ast_fn.name() else {
        return Ok(None);
    };
    let cc = count_cc(ast_fn.syntax());
    let analysis = host.analysis();
    let file_id = real_file_id.file_id(db);
    let line_index = analysis.file_line_index(file_id)?;
    let line = line_index.line_col(ast_fn.syntax().text_range().start()).line;
    let path = vfs
        .file_path(file_id)
        .as_path()
        .map(|p| p.to_string())
        .unwrap_or_default();
    let mut scope = scope_chain.join("::");
    if !scope.is_empty() {
        scope.push_str("::");
    }
    scope.push_str(&name.text());
    Ok(Some(CcRow {
        file: path,
        line,
        scope,
        cc,
    }))
}

/// Walks the function body and counts decision points using the
/// same rules as the syn-based CC visitor.
fn count_cc(node: &SyntaxNode) -> u32 {
    let mut cc = 1; // baseline
    for desc in node.descendants() {
        cc += kind_contribution(desc);
    }
    cc
}

/// Per-node CC contribution. Pulled out so the kind dispatch stays
/// flat and self-application's match-arm-count / CC gates pass.
fn kind_contribution(desc: SyntaxNode) -> u32 {
    if matches!(
        desc.kind(),
        SyntaxKind::IF_EXPR
            | SyntaxKind::WHILE_EXPR
            | SyntaxKind::FOR_EXPR
            | SyntaxKind::LOOP_EXPR
            | SyntaxKind::TRY_EXPR
    ) {
        return 1;
    }
    if desc.kind() == SyntaxKind::MATCH_EXPR {
        return ast::MatchExpr::cast(desc)
            .map(|m| match_arms_contribution(&m))
            .unwrap_or(0);
    }
    if desc.kind() == SyntaxKind::BIN_EXPR {
        return ast::BinExpr::cast(desc).map(bin_logic_contribution).unwrap_or(0);
    }
    0
}

fn bin_logic_contribution(b: ast::BinExpr) -> u32 {
    matches!(
        b.op_kind(),
        Some(BinaryOp::LogicOp(LogicOp::And)) | Some(BinaryOp::LogicOp(LogicOp::Or))
    ) as u32
}

/// Sealed-aware: a `match` whose arm list contains a wildcard `_`
/// pattern contributes (arm_count - 1); otherwise the compiler is
/// enforcing exhaustiveness and it contributes 0.
fn match_arms_contribution(m: &ast::MatchExpr) -> u32 {
    let Some(arm_list) = m.match_arm_list() else {
        return 0;
    };
    let arms: Vec<_> = arm_list.arms().collect();
    if arms.is_empty() {
        return 0;
    }
    let has_wildcard = arms
        .iter()
        .any(|a| a.pat().is_some_and(|p| matches!(p, Pat::WildcardPat(_))));
    if has_wildcard {
        (arms.len() as u32).saturating_sub(1)
    } else {
        0
    }
}
