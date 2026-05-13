//! HIR-aware function-level complexity lenses.
//!
//! Computes Cyclomatic Complexity (McCabe 1976), Cognitive
//! Complexity (Campbell / Sonar 2018), and NPath Complexity
//! (Nejmeh 1988) per function with two HIR refinements over the
//! `ra_ap_syntax`-only implementations:
//!
//! 1. **Sealed-match exactness.** The AST lens counts a `match`
//!    with no wildcard arm as zero contribution under the
//!    assumption the compiler enforces exhaustiveness. That holds
//!    when the scrutinee is an enum, but the AST walker can't tell
//!    an enum scrutinee from a `bool` / numeric / string scrutinee
//!    that also lacks a `_` arm — those still represent runtime
//!    decision points. The HIR walker asks `Semantics::type_of_expr`
//!    and applies the sealed rule only when the type resolves to a
//!    `hir::Adt::Enum`.
//! 2. **Cognitive B1 direct recursion.** Sonar's B1 rule charges
//!    +1 per direct recursive call. The AST lens currently misses
//!    this entirely. The HIR walker resolves each call expression
//!    inside the function body and credits a +1 when the callee
//!    resolves back to the enclosing function — through any path
//!    shape (`foo()`, `Self::foo()`, `crate::foo::foo()`, etc.).
//!
//! All three lenses share the per-file walking infrastructure
//! (workspace load, file enumeration, AST traversal). The output
//! is one [`FunctionComplexity`] per function carrying all three
//! values.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use ra_ap_hir::{attach_db, Crate, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::{
    ast::{self, AstNode, BinaryOp, HasName, LogicOp, Pat},
    SyntaxKind, SyntaxNode,
};

/// One function's complexity triple.
#[derive(Debug, Clone)]
pub struct FunctionComplexity {
    /// VFS-shaped file path (forward slashes, absolute).
    pub file: String,
    /// Scope path — `module::Type::method` or `module::fn`.
    pub scope: String,
    /// 1-based start line of the function declaration.
    pub line: u32,
    /// McCabe Cyclomatic Complexity (sealed-match HIR-refined).
    pub cyclomatic: u32,
    /// Sonar Cognitive Complexity (B1+B2+B3, with direct-recursion +1).
    pub cognitive: u32,
    /// Nejmeh NPath (sealed-match HIR-refined; capped at 1e9).
    pub npath: f64,
}

/// Loads `manifest_dir`, walks every workspace-local source file
/// for `ast::Fn` items, and emits one [`FunctionComplexity`] per
/// function.
pub fn detect_at(manifest_dir: &Path) -> Result<Vec<FunctionComplexity>> {
    let workspace = crate::workspace::load(manifest_dir)?;
    let db = workspace.host.raw_database();
    let vfs = &workspace.vfs;
    attach_db(db, || -> Result<Vec<FunctionComplexity>> {
        let files = enumerate_files(db, vfs);
        let sema = Semantics::new(db);
        let mut out: Vec<FunctionComplexity> = Vec::new();
        for entry in files.values() {
            let parsed = sema.parse_guess_edition(entry.file_id);
            walk_file(&sema, &parsed, entry, &mut out);
        }
        out.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then_with(|| a.line.cmp(&b.line))
                .then_with(|| a.scope.cmp(&b.scope))
        });
        Ok(out)
    })
}

#[derive(Clone)]
struct FileEntry {
    path: String,
    file_id: ra_ap_vfs::FileId,
}

fn enumerate_files(db: &RootDatabase, vfs: &ra_ap_vfs::Vfs) -> HashMap<u32, FileEntry> {
    let mut out: HashMap<u32, FileEntry> = HashMap::new();
    for krate in Crate::all(db) {
        if !krate.origin(db).is_local() {
            continue;
        }
        let mut stack = vec![krate.root_module(db)];
        while let Some(module) = stack.pop() {
            for child in module.children(db) {
                stack.push(child);
            }
            let Some(editioned) = module.as_source_file_id(db) else {
                continue;
            };
            let file_id = editioned.file_id(db);
            let key = file_id.index();
            if out.contains_key(&key) {
                continue;
            }
            let Some(path) = vfs.file_path(file_id).as_path().map(|p| p.to_string()) else {
                continue;
            };
            out.insert(key, FileEntry { path, file_id });
        }
    }
    out
}

fn walk_file(
    sema: &Semantics<'_, RootDatabase>,
    parsed: &ast::SourceFile,
    entry: &FileEntry,
    out: &mut Vec<FunctionComplexity>,
) {
    for node in parsed.syntax().descendants() {
        let Some(fn_) = ast::Fn::cast(node) else {
            continue;
        };
        let Some(measurement) = measure_function(sema, &fn_, entry) else {
            continue;
        };
        out.push(measurement);
    }
}

fn measure_function(
    sema: &Semantics<'_, RootDatabase>,
    ast_fn: &ast::Fn,
    entry: &FileEntry,
) -> Option<FunctionComplexity> {
    let body = ast_fn.body()?;
    let body_syntax = body.syntax();
    let fn_hir = sema.to_def(ast_fn);
    let scope = derive_scope_path(sema, ast_fn);
    let line = line_of(ast_fn.syntax());

    let cyclomatic = count_cc(sema, body_syntax);
    let cognitive = count_cognitive(sema, body_syntax, fn_hir);
    let npath = count_npath(sema, body_syntax);

    Some(FunctionComplexity {
        file: entry.path.clone(),
        scope,
        line,
        cyclomatic,
        cognitive,
        npath,
    })
}

/// Reconstructs a `module::Type::method` path for the function. Uses
/// the AST ancestor chain: walk up through Modules and ImplBlocks
/// collecting their names; reverse and join.
fn derive_scope_path(sema: &Semantics<'_, RootDatabase>, ast_fn: &ast::Fn) -> String {
    let _ = sema; // currently unused; reserved for future HIR scope lookup
    let mut parts: Vec<String> = Vec::new();
    if let Some(name) = ast_fn.name() {
        parts.push(name.text().to_string());
    } else {
        parts.push("<unnamed>".into());
    }
    let mut node = ast_fn.syntax().parent();
    while let Some(n) = node {
        if let Some(impl_) = ast::Impl::cast(n.clone()) {
            if let Some(ty) = impl_.self_ty() {
                parts.push(ty.syntax().text().to_string());
            }
        } else if let Some(module) = ast::Module::cast(n.clone()) {
            if let Some(name) = module.name() {
                parts.push(name.text().to_string());
            }
        }
        node = n.parent();
    }
    parts.reverse();
    parts.join("::")
}

fn line_of(node: &SyntaxNode) -> u32 {
    let offset: usize = node.text_range().start().into();
    let text = node
        .ancestors()
        .last()
        .map(|root| root.text().to_string())
        .unwrap_or_default();
    let line = text[..offset.min(text.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count();
    (line as u32) + 1
}

// =========================================================================
// Cyclomatic Complexity (McCabe 1976) with HIR-refined sealed-match
// =========================================================================

fn count_cc(sema: &Semantics<'_, RootDatabase>, body: &SyntaxNode) -> u32 {
    let mut cc = 1;
    for desc in body.descendants() {
        cc += cc_node_contribution(sema, &desc);
    }
    cc
}

fn cc_node_contribution(sema: &Semantics<'_, RootDatabase>, node: &SyntaxNode) -> u32 {
    match node.kind() {
        SyntaxKind::IF_EXPR
        | SyntaxKind::WHILE_EXPR
        | SyntaxKind::FOR_EXPR
        | SyntaxKind::LOOP_EXPR
        | SyntaxKind::TRY_EXPR => 1,
        SyntaxKind::MATCH_EXPR => ast::MatchExpr::cast(node.clone())
            .map(|m| match_contribution(sema, &m))
            .unwrap_or(0),
        SyntaxKind::BIN_EXPR => ast::BinExpr::cast(node.clone())
            .map(bin_logic_contribution)
            .unwrap_or(0),
        _ => 0,
    }
}

/// HIR-refined sealed-match: a `match` with no wildcard contributes
/// `0` only when the scrutinee's type is an enum (the compiler
/// enforces exhaustiveness there). For non-enum scrutinees
/// (`bool`, numeric, string, struct destructures, …) every arm
/// beyond the first remains a runtime decision point.
fn match_contribution(sema: &Semantics<'_, RootDatabase>, m: &ast::MatchExpr) -> u32 {
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
        return (arms.len() as u32).saturating_sub(1);
    }
    if scrutinee_is_enum(sema, m) {
        // Sealed enum match — compiler-enforced exhaustiveness.
        return 0;
    }
    // Non-enum scrutinee without `_` — every arm is still a
    // runtime branch.
    (arms.len() as u32).saturating_sub(1)
}

fn scrutinee_is_enum(sema: &Semantics<'_, RootDatabase>, m: &ast::MatchExpr) -> bool {
    let Some(scrutinee) = m.expr() else {
        return false;
    };
    let Some(ty_info) = sema.type_of_expr(&scrutinee) else {
        return false;
    };
    // `TypeInfo::adjusted` takes self; capture `original` first
    // so we can fall back if the adjusted type is not an ADT.
    let original_adt = ty_info.original.as_adt();
    let adjusted_adt = ty_info.adjusted.and_then(|t| t.as_adt());
    let adt = match adjusted_adt.or(original_adt) {
        Some(a) => a,
        None => return false,
    };
    matches!(adt, ra_ap_hir::Adt::Enum(_))
}

fn bin_logic_contribution(b: ast::BinExpr) -> u32 {
    matches!(
        b.op_kind(),
        Some(BinaryOp::LogicOp(LogicOp::And)) | Some(BinaryOp::LogicOp(LogicOp::Or))
    ) as u32
}

// =========================================================================
// Cognitive Complexity (Sonar 2018) with B1 direct recursion
// =========================================================================

fn count_cognitive(
    sema: &Semantics<'_, RootDatabase>,
    body: &SyntaxNode,
    enclosing: Option<ra_ap_hir::Function>,
) -> u32 {
    walk_cognitive(sema, body, 0, enclosing)
}

fn walk_cognitive(
    sema: &Semantics<'_, RootDatabase>,
    node: &SyntaxNode,
    depth: u32,
    enclosing: Option<ra_ap_hir::Function>,
) -> u32 {
    let mut total = 0u32;
    for child in node.children() {
        let (penalty, next_depth) = cognitive_node_penalty(sema, &child, depth, enclosing);
        total += penalty;
        total += walk_cognitive(sema, &child, next_depth, enclosing);
    }
    total
}

fn cognitive_node_penalty(
    sema: &Semantics<'_, RootDatabase>,
    node: &SyntaxNode,
    depth: u32,
    enclosing: Option<ra_ap_hir::Function>,
) -> (u32, u32) {
    match node.kind() {
        SyntaxKind::IF_EXPR
        | SyntaxKind::WHILE_EXPR
        | SyntaxKind::FOR_EXPR
        | SyntaxKind::LOOP_EXPR
        | SyntaxKind::MATCH_EXPR => (1 + depth, depth + 1),
        SyntaxKind::BIN_EXPR => {
            let p = ast::BinExpr::cast(node.clone())
                .map(bin_logic_contribution)
                .unwrap_or(0);
            (p, depth)
        }
        SyntaxKind::CLOSURE_EXPR => (0, 0),
        SyntaxKind::CALL_EXPR => {
            let p = ast::CallExpr::cast(node.clone())
                .and_then(|c| recursion_bonus(sema, c.syntax(), enclosing))
                .unwrap_or(0);
            (p, depth)
        }
        SyntaxKind::METHOD_CALL_EXPR => {
            let p = recursion_bonus(sema, node, enclosing).unwrap_or(0);
            (p, depth)
        }
        _ => (0, depth),
    }
}

/// Returns `Some(1)` when the call resolves to the enclosing
/// function (direct recursion); `None` otherwise. Covers free-fn
/// calls (`foo()`), method calls (`self.foo()`), and path-call
/// shapes (`Self::foo(self)`, `crate::foo::foo()`).
fn recursion_bonus(
    sema: &Semantics<'_, RootDatabase>,
    call_node: &SyntaxNode,
    enclosing: Option<ra_ap_hir::Function>,
) -> Option<u32> {
    let enclosing = enclosing?;
    let callee = resolve_callee(sema, call_node)?;
    if callee == enclosing {
        Some(1)
    } else {
        None
    }
}

fn resolve_callee(
    sema: &Semantics<'_, RootDatabase>,
    call_node: &SyntaxNode,
) -> Option<ra_ap_hir::Function> {
    if let Some(call) = ast::CallExpr::cast(call_node.clone()) {
        let path_expr = ast::PathExpr::cast(call.expr()?.syntax().clone())?;
        let path = path_expr.path()?;
        let resolution = sema.resolve_path(&path)?;
        if let ra_ap_hir::PathResolution::Def(ra_ap_hir::ModuleDef::Function(f)) = resolution {
            return Some(f);
        }
    } else if let Some(mc) = ast::MethodCallExpr::cast(call_node.clone()) {
        return sema.resolve_method_call(&mc);
    }
    None
}

// =========================================================================
// NPath Complexity (Nejmeh 1988) with HIR-refined sealed-match
// =========================================================================

fn count_npath(sema: &Semantics<'_, RootDatabase>, body: &SyntaxNode) -> f64 {
    npath(sema, body)
}

fn npath(sema: &Semantics<'_, RootDatabase>, node: &SyntaxNode) -> f64 {
    let mut acc = 1.0_f64;
    for child in node.children() {
        acc *= npath_factor(sema, &child);
    }
    if acc > 1e9 {
        1e9
    } else {
        acc
    }
}

fn product_of_children(sema: &Semantics<'_, RootDatabase>, node: &SyntaxNode) -> f64 {
    let mut acc = 1.0_f64;
    for child in node.children() {
        acc *= npath_factor(sema, &child);
    }
    if acc < 1.0 {
        1.0
    } else {
        acc
    }
}

fn npath_factor(sema: &Semantics<'_, RootDatabase>, node: &SyntaxNode) -> f64 {
    match node.kind() {
        SyntaxKind::IF_EXPR => {
            let if_ = ast::IfExpr::cast(node.clone());
            let then_n = if_
                .as_ref()
                .and_then(|i| i.then_branch())
                .map(|b| npath(sema, b.syntax()))
                .unwrap_or(1.0);
            let else_n = if_
                .as_ref()
                .and_then(|i| i.else_branch())
                .map(|e| match e {
                    ast::ElseBranch::Block(b) => npath(sema, b.syntax()),
                    ast::ElseBranch::IfExpr(ie) => npath_factor(sema, ie.syntax()),
                })
                .unwrap_or(1.0);
            then_n + else_n
        }
        SyntaxKind::MATCH_EXPR => {
            let m = ast::MatchExpr::cast(node.clone());
            // Sealed-aware: an exhaustive enum match contributes
            // 1 (the compiler enforces coverage; only one branch
            // executes), matching the CC rule. Other no-wildcard
            // matches treat each arm as a real branch.
            if let Some(m) = m.as_ref() {
                let arms = m
                    .match_arm_list()
                    .map(|al| al.arms().count())
                    .unwrap_or(0)
                    .max(1);
                let has_wildcard = m
                    .match_arm_list()
                    .into_iter()
                    .flat_map(|al| al.arms())
                    .any(|a| a.pat().is_some_and(|p| matches!(p, Pat::WildcardPat(_))));
                if !has_wildcard && scrutinee_is_enum(sema, m) {
                    return 1.0;
                }
                return arms as f64;
            }
            1.0
        }
        SyntaxKind::WHILE_EXPR | SyntaxKind::FOR_EXPR | SyntaxKind::LOOP_EXPR => {
            product_of_children(sema, node) + 1.0
        }
        _ => product_of_children(sema, node),
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn unique_dir(label: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let s = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("rustics-ra-fc-{label}-{pid}-{n}-{s}"))
    }

    fn write_fixture(dir: &std::path::Path, lib_rs: &str) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"fc\"\nversion = \"0.0.1\"\nedition = \"2021\"\npublish = false\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
        fs::write(dir.join("src/lib.rs"), lib_rs).unwrap();
    }

    fn find<'a>(out: &'a [FunctionComplexity], scope: &str) -> &'a FunctionComplexity {
        out.iter()
            .find(|m| m.scope == scope)
            .unwrap_or_else(|| panic!("no measurement for {scope}; got: {out:?}"))
    }

    #[test]
    fn sealed_enum_match_charges_zero_for_cc_and_one_for_npath() {
        // HIR-refined sealed match: scrutinee is enum, no wildcard
        // → CC 1 (baseline only), NPath 1.
        let dir = unique_dir("sealed-enum");
        write_fixture(
            &dir,
            "pub enum E { A, B } pub fn f(e: E) -> i32 { match e { E::A => 1, E::B => 2 } }\n",
        );
        let out = detect_at(&dir).expect("detect_at");
        let f = find(&out, "f");
        assert_eq!(f.cyclomatic, 1, "sealed enum match adds nothing: {f:?}");
        assert!((f.npath - 1.0).abs() < 1e-9, "sealed enum npath = 1: {f:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bool_match_without_wildcard_does_not_get_sealed_treatment() {
        // The AST walker would treat this as sealed (no wildcard)
        // and charge 0. HIR resolves the scrutinee as `bool` (not
        // an enum), so the arms count as real branches.
        let dir = unique_dir("bool-match");
        write_fixture(
            &dir,
            "pub fn f(x: bool) -> i32 { match x { true => 1, false => 0 } }\n",
        );
        let out = detect_at(&dir).expect("detect_at");
        let f = find(&out, "f");
        assert_eq!(f.cyclomatic, 2, "bool match arms count as branches: {f:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn direct_recursion_adds_one_to_cognitive() {
        // Cognitive B1: a call resolving to the enclosing fn = +1.
        // No control-flow constructs, just one recursive call.
        let dir = unique_dir("recursive");
        write_fixture(
            &dir,
            "pub fn factorial(n: u32) -> u32 { factorial(n - 1) }\n",
        );
        let out = detect_at(&dir).expect("detect_at");
        let f = find(&out, "factorial");
        assert_eq!(
            f.cognitive, 1,
            "direct recursion adds B1 +1 even with no control flow: {f:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
