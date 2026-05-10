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

use ra_ap_hir::{attach_db, AssocItem, Crate, HasCrate, HasSource, Impl, Module, ModuleDef};
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

/// Top-level entry point: open the workspace, walk every workspace-
/// member crate, compute CC per function, return rows.
pub fn measure_at(manifest_dir: &Path) -> Result<Vec<CcRow>> {
    let workspace = crate::workspace::load(manifest_dir)?;
    let db = workspace.host.raw_database();
    let host = &workspace.host;
    let vfs = &workspace.vfs;
    attach_db(db, || -> Result<Vec<CcRow>> {
        let mut out = Vec::new();
        for krate in Crate::all(db) {
            if !krate.origin(db).is_local() {
                continue;
            }
            let root = krate.root_module(db);
            walk_module(db, root, host, vfs, &mut out, &mut Vec::new())?;
        }
        Ok(out)
    })
}

fn walk_module(
    db: &ra_ap_ide_db::RootDatabase,
    module: Module,
    host: &ra_ap_ide::AnalysisHost,
    vfs: &ra_ap_vfs::Vfs,
    out: &mut Vec<CcRow>,
    scope_chain: &mut Vec<String>,
) -> Result<()> {
    let pushed = push_module_name(db, module, scope_chain);
    for decl in module.declarations(db) {
        process_decl(db, decl, host, vfs, out, scope_chain)?;
    }
    for child in module.children(db) {
        walk_module(db, child, host, vfs, out, scope_chain)?;
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
) -> Result<()> {
    match decl {
        ModuleDef::Function(f) => {
            if let Some(row) = measure_function(db, f, host, vfs, scope_chain)? {
                out.push(row);
            }
        }
        ModuleDef::Adt(_) | ModuleDef::Trait(_) => {
            walk_associated_fns(db, decl, host, vfs, out, scope_chain)?;
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
) -> Result<()> {
    let Some((parent_name, fns)) = collect_assoc_fns(db, decl) else {
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
/// Adt's inherent impls are gathered crate-wide via `Impl::all_in_crate`.
fn collect_assoc_fns(
    db: &ra_ap_ide_db::RootDatabase,
    decl: ModuleDef,
) -> Option<(String, Vec<ra_ap_hir::Function>)> {
    match decl {
        ModuleDef::Adt(a) => Some((a.name(db).as_str().to_string(), adt_inherent_fns(db, a))),
        ModuleDef::Trait(t) => Some((t.name(db).as_str().to_string(), trait_fns(db, t))),
        _ => None,
    }
}

fn adt_inherent_fns(
    db: &ra_ap_ide_db::RootDatabase,
    a: ra_ap_hir::Adt,
) -> Vec<ra_ap_hir::Function> {
    let krate = a.krate(db);
    let mut fns = Vec::new();
    for imp in Impl::all_in_crate(db, krate) {
        if !impl_targets_adt(db, imp, a) {
            continue;
        }
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
/// Returns `true` when `imp.self_ty(db)` resolves to `adt`. Used to
/// filter the crate-wide `Impl::all_in_crate` list down to the
/// impls of one specific Adt. (The HIR API does not expose a
/// per-Adt `impls()` accessor.)
fn impl_targets_adt(db: &ra_ap_ide_db::RootDatabase, imp: Impl, adt: ra_ap_hir::Adt) -> bool {
    let self_ty = imp.self_ty(db);
    self_ty.as_adt() == Some(adt)
}

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
