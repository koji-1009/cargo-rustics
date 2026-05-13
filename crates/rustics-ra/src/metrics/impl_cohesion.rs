//! HIR-aware impl-block cohesion lenses.
//!
//! Computes LCOM4 (Hitz & Montazeri 1995), RFC (Chidamber & Kemerer
//! 1994), and WMC (Chidamber & Kemerer 1994) per inherent `impl`
//! block with the HIR refinements the spike triage flagged:
//!
//! 1. **Aliased `self` in LCOM4.** `let s = self; s.field` resolves
//!    to the same field-share edge as `self.field`. The AST walker
//!    keys on the bare keyword `self` and misses the binding.
//! 2. **Qualified self-method calls.** `Self::method(self)` and
//!    `<Self as Trait>::method(self)` resolve to the same method
//!    `M`; the AST walker only sees the `self.method()` shape.
//! 3. **RFC method-vs-free-function disambiguation.** `module::
//!    helper()` (free function) and `Type::associated_fn()` (method)
//!    are syntactically identical; the AST walker counts both as
//!    method-message dispatch, inflating R. HIR resolves each call
//!    to a `hir::Function` whose parent we can check.
//!
//! Trait impls are skipped — the method set there is dictated by
//! the trait contract, not the type's own cohesion choice. That
//! matches both the AST walker's behaviour and the rationale in
//! the existing lens metadata.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use ra_ap_hir::{attach_db, AsAssocItem, Crate, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::{
    ast::{self, AstNode, BinaryOp, HasName as _, LogicOp, Pat},
    SyntaxKind, SyntaxNode,
};

/// One inherent-impl cohesion triple, keyed by file + the impl's
/// self-type name + the impl-block's start line. Multiple `impl T
/// { … }` blocks for the same `T` emit one measurement each (one
/// per syntactic block) — the AST lens does the same.
#[derive(Debug, Clone)]
pub struct ImplCohesion {
    /// VFS-shaped file path (forward slashes, absolute).
    pub file: String,
    /// Self-type display name (`Foo`, `Foo<T>` truncated to `Foo`).
    pub scope: String,
    /// 1-based start line of the `impl` block.
    pub line: u32,
    /// Number of methods in the impl block (size of `M`).
    pub method_count: u32,
    /// LCOM4: connected components in the method graph. `None`
    /// when the impl has fewer than two methods (LCOM4 needs ≥ 2
    /// methods to make a claim).
    pub lcom4: Option<u32>,
    /// RFC: `|M ∪ R|`. `R` is the distinct method-call set
    /// resolved through HIR; free-function calls are excluded.
    pub rfc: u32,
    /// WMC: sum of HIR-refined cyclomatic complexity across the
    /// impl's methods.
    pub wmc: u32,
}

/// Loads `manifest_dir`, walks every workspace-local inherent impl
/// block, and emits one [`ImplCohesion`] per block. Trait impls,
/// blanket impls, and impls on non-ADT types (e.g. tuples) are
/// skipped — the cohesion question doesn't apply there.
pub fn detect_at(manifest_dir: &Path) -> Result<Vec<ImplCohesion>> {
    let workspace = crate::workspace::load(manifest_dir)?;
    let db = workspace.host.raw_database();
    let vfs = &workspace.vfs;
    attach_db(db, || -> Result<Vec<ImplCohesion>> {
        let files = enumerate_files(db, vfs);
        let sema = Semantics::new(db);
        let mut out = Vec::new();
        for entry in files.values() {
            let parsed = sema.parse_guess_edition(entry.file_id);
            for node in parsed.syntax().descendants() {
                let Some(ast_impl) = ast::Impl::cast(node) else {
                    continue;
                };
                // Trait impls are filtered through the HIR handle
                // — we still need the handle to call HIR resolution
                // on the body, so look it up via Semantics here
                // rather than walking `module.impl_defs(db)`.
                let Some(hir_impl) = sema.to_def(&ast_impl) else {
                    continue;
                };
                if hir_impl.trait_(db).is_some() {
                    continue;
                }
                if let Some(measurement) = measure_impl(db, &sema, &ast_impl, hir_impl, &entry.path)
                {
                    out.push(measurement);
                }
            }
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

fn enumerate_files(
    db: &RootDatabase,
    vfs: &ra_ap_vfs::Vfs,
) -> std::collections::HashMap<u32, FileEntry> {
    let mut out: std::collections::HashMap<u32, FileEntry> = std::collections::HashMap::new();
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

fn measure_impl(
    db: &RootDatabase,
    sema: &Semantics<'_, RootDatabase>,
    ast_impl: &ast::Impl,
    hir_impl: ra_ap_hir::Impl,
    file_path: &str,
) -> Option<ImplCohesion> {
    let scope = impl_self_ty_name(db, hir_impl)?;
    let line = line_of(ast_impl.syntax());
    let methods = collect_methods(db, sema, ast_impl, hir_impl);
    if methods.is_empty() {
        return None;
    }
    let method_count = methods.len() as u32;
    let lcom4 = if methods.len() < 2 {
        None
    } else {
        Some(connected_components(&methods))
    };
    let rfc = compute_rfc(&methods);
    let wmc = methods.iter().map(|m| m.cc).sum();
    Some(ImplCohesion {
        file: file_path.to_string(),
        scope,
        line,
        method_count,
        lcom4,
        rfc,
        wmc,
    })
}

fn impl_self_ty_name(db: &RootDatabase, impl_def: ra_ap_hir::Impl) -> Option<String> {
    let adt = impl_def.self_ty(db).as_adt()?;
    Some(adt.name(db).as_str().to_owned())
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

struct MethodInfo {
    /// Method name as it appears in the source — the AST lens
    /// keyed its method graph on this string. We preserve it for
    /// edge matching; HIR resolution upgrades the *receiver* check
    /// but the call's target name still uses the textual shape.
    name: String,
    /// `hir::Function` handle for the method. Used to compare
    /// against resolved callees (so `Self::foo(self)` and
    /// `<Self as Trait>::foo(self)` collapse to one edge).
    hir_fn: Option<ra_ap_hir::Function>,
    /// Self-field access set (the field's name) — built via type
    /// resolution rather than literal-`self` matching, so aliased
    /// `let s = self; s.field` joins the same edge as `self.field`.
    fields: HashSet<String>,
    /// Self-method-call set (callee function handle). HIR resolves
    /// path-call shapes (`Self::foo()`) into the same handle as
    /// method-call shapes (`self.foo()`), so the union is exact.
    self_callees: HashSet<ra_ap_hir::Function>,
    /// RFC `R` contribution from this method's body. Already a
    /// set; the caller folds these into a single global R, then
    /// adds `M`.
    rfc_callees: HashSet<ra_ap_hir::Function>,
    /// CC contribution to WMC. Uses the same sealed-match rule the
    /// HIR `function_complexity` walker uses, modulo recursion
    /// detection (CC's McCabe definition doesn't include
    /// recursion).
    cc: u32,
}

fn collect_methods(
    db: &RootDatabase,
    sema: &Semantics<'_, RootDatabase>,
    ast_impl: &ast::Impl,
    hir_impl: ra_ap_hir::Impl,
) -> Vec<MethodInfo> {
    let self_ty = hir_impl.self_ty(db);
    let Some(items) = ast_impl.assoc_item_list() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items.assoc_items() {
        let ast::AssocItem::Fn(ast_fn) = item else {
            continue;
        };
        let Some(method) = build_method_info(sema, &ast_fn, &self_ty) else {
            continue;
        };
        out.push(method);
    }
    out
}

fn build_method_info(
    sema: &Semantics<'_, RootDatabase>,
    ast_fn: &ast::Fn,
    self_ty: &ra_ap_hir::Type<'_>,
) -> Option<MethodInfo> {
    let name = ast_fn.name()?.text().to_string();
    let hir_fn = sema.to_def(ast_fn);
    let body_syntax = ast_fn.body().map(|b| b.syntax().clone());
    let mut fields: HashSet<String> = HashSet::new();
    let mut self_callees: HashSet<ra_ap_hir::Function> = HashSet::new();
    let mut rfc_callees: HashSet<ra_ap_hir::Function> = HashSet::new();
    if let Some(body) = &body_syntax {
        walk_body(
            sema,
            body,
            self_ty,
            &mut fields,
            &mut self_callees,
            &mut rfc_callees,
        );
    }
    let cc = body_syntax.as_ref().map(|b| count_cc(sema, b)).unwrap_or(1);
    Some(MethodInfo {
        name,
        hir_fn,
        fields,
        self_callees,
        rfc_callees,
        cc,
    })
}

fn walk_body(
    sema: &Semantics<'_, RootDatabase>,
    body: &SyntaxNode,
    self_ty: &ra_ap_hir::Type<'_>,
    fields: &mut HashSet<String>,
    self_callees: &mut HashSet<ra_ap_hir::Function>,
    rfc_callees: &mut HashSet<ra_ap_hir::Function>,
) {
    for desc in body.descendants() {
        match desc.kind() {
            SyntaxKind::FIELD_EXPR => {
                if let Some(fe) = ast::FieldExpr::cast(desc.clone()) {
                    record_field(sema, &fe, self_ty, fields);
                }
            }
            SyntaxKind::METHOD_CALL_EXPR => {
                if let Some(mc) = ast::MethodCallExpr::cast(desc.clone()) {
                    record_method_call(sema, &mc, self_ty, self_callees, rfc_callees);
                }
            }
            SyntaxKind::CALL_EXPR => {
                if let Some(c) = ast::CallExpr::cast(desc.clone()) {
                    record_path_call(sema, &c, self_ty, self_callees, rfc_callees);
                }
            }
            _ => {}
        }
    }
}

/// HIR-refined field-share: record a field name only when the
/// receiver's type matches the impl's self type. Covers `self.f`,
/// `(*self).f`, and aliased `let s = self; s.f`.
fn record_field(
    sema: &Semantics<'_, RootDatabase>,
    fe: &ast::FieldExpr,
    self_ty: &ra_ap_hir::Type<'_>,
    fields: &mut HashSet<String>,
) {
    let Some(receiver) = fe.expr() else { return };
    if !receiver_type_is_self(sema, &receiver, self_ty) {
        return;
    }
    if let Some(name) = fe.name_ref() {
        fields.insert(name.text().to_string());
    }
}

/// HIR-refined method-call edge: a call on a receiver whose type
/// is the impl's self type. The resolved callee `Function` goes
/// into `self_callees` for LCOM4's cohesion graph; the same
/// `Function` also enters `rfc_callees` so RFC counts it.
fn record_method_call(
    sema: &Semantics<'_, RootDatabase>,
    mc: &ast::MethodCallExpr,
    self_ty: &ra_ap_hir::Type<'_>,
    self_callees: &mut HashSet<ra_ap_hir::Function>,
    rfc_callees: &mut HashSet<ra_ap_hir::Function>,
) {
    let Some(callee) = sema.resolve_method_call(mc) else {
        return;
    };
    rfc_callees.insert(callee);
    let Some(receiver) = mc.receiver() else {
        return;
    };
    if receiver_type_is_self(sema, &receiver, self_ty) {
        self_callees.insert(callee);
    }
}

/// Path-call resolution: `Self::method(self)`, `<Self as
/// Trait>::method(self)`, or any qualified `Type::method(...)`
/// shape. RFC counts the call only when the callee resolves to a
/// `Function` whose parent is an `Impl` or `Trait` (method
/// dispatch). Free-function calls don't contribute to RFC under
/// CK's original definition.
fn record_path_call(
    sema: &Semantics<'_, RootDatabase>,
    call: &ast::CallExpr,
    self_ty: &ra_ap_hir::Type<'_>,
    self_callees: &mut HashSet<ra_ap_hir::Function>,
    rfc_callees: &mut HashSet<ra_ap_hir::Function>,
) {
    let Some(expr) = call.expr() else { return };
    let ast::Expr::PathExpr(path_expr) = expr else {
        return;
    };
    let Some(path) = path_expr.path() else { return };
    let Some(resolution) = sema.resolve_path(&path) else {
        return;
    };
    let ra_ap_hir::PathResolution::Def(ra_ap_hir::ModuleDef::Function(f)) = resolution else {
        return;
    };
    // Free function vs method: an associated fn lives in an Impl
    // or Trait; a free fn lives in a Module. Only the former
    // counts toward RFC.
    if !is_associated_function(sema.db, f) {
        return;
    }
    rfc_callees.insert(f);
    // Self-association: the first segment (`Self::…`,
    // `<Self as Trait>::…`) carries a path that resolves to a
    // type matching `self_ty`. We approximate by treating any
    // path-call to a function whose parent type equals the impl's
    // self type as a self-call.
    if function_belongs_to_self_ty(sema.db, f, self_ty) {
        self_callees.insert(f);
    }
}

fn is_associated_function(db: &dyn ra_ap_hir::db::HirDatabase, f: ra_ap_hir::Function) -> bool {
    matches!(f.as_assoc_item(db), Some(ra_ap_hir::AssocItem::Function(_)))
}

fn function_belongs_to_self_ty(
    db: &dyn ra_ap_hir::db::HirDatabase,
    f: ra_ap_hir::Function,
    self_ty: &ra_ap_hir::Type<'_>,
) -> bool {
    let Some(assoc_container) = f.as_assoc_item(db).map(|a| a.container(db)) else {
        return false;
    };
    let ra_ap_hir::AssocItemContainer::Impl(impl_def) = assoc_container else {
        return false;
    };
    // Compare ADTs; self_ty's Type can be compared structurally
    // through `as_adt`.
    match (impl_def.self_ty(db).as_adt(), self_ty.as_adt()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Returns `true` when `receiver`'s resolved type matches the
/// impl's self type. Handles aliased self (`let s = self; …`),
/// dereferenced self (`(*self).field`), and `&self` automatically
/// because `Semantics::type_of_expr` adjusts for those.
fn receiver_type_is_self(
    sema: &Semantics<'_, RootDatabase>,
    receiver: &ast::Expr,
    self_ty: &ra_ap_hir::Type<'_>,
) -> bool {
    let Some(ty_info) = sema.type_of_expr(receiver) else {
        return false;
    };
    let target = self_ty.as_adt();
    if target.is_none() {
        return false;
    }
    if peel_to_adt(&ty_info.original) == target {
        return true;
    }
    if let Some(adjusted) = ty_info.adjusted {
        if peel_to_adt(&adjusted) == target {
            return true;
        }
    }
    false
}

/// Walks through `&T`, `&mut T`, `Box<T>`-style references to reach
/// the underlying ADT. `&self` receivers carry type `&S`, not `S`;
/// without this peel, the as_adt comparison misses the obvious
/// self-method-call edge.
fn peel_to_adt(ty: &ra_ap_hir::Type<'_>) -> Option<ra_ap_hir::Adt> {
    if let Some(inner) = ty.remove_ref() {
        return peel_to_adt(&inner);
    }
    ty.as_adt()
}

/// CC counting that mirrors `function_complexity`'s sealed-match
/// rule. Kept local so this module doesn't take an `impl_cohesion`
/// → `function_complexity` dependency for what amounts to one
/// shared helper.
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
        return 0;
    }
    (arms.len() as u32).saturating_sub(1)
}

fn scrutinee_is_enum(sema: &Semantics<'_, RootDatabase>, m: &ast::MatchExpr) -> bool {
    let Some(scrutinee) = m.expr() else {
        return false;
    };
    let Some(ty_info) = sema.type_of_expr(&scrutinee) else {
        return false;
    };
    let original_adt = ty_info.original.as_adt();
    let adjusted_adt = ty_info.adjusted.and_then(|t| t.as_adt());
    let adt = adjusted_adt.or(original_adt);
    matches!(adt, Some(ra_ap_hir::Adt::Enum(_)))
}

fn bin_logic_contribution(b: ast::BinExpr) -> u32 {
    matches!(
        b.op_kind(),
        Some(BinaryOp::LogicOp(LogicOp::And)) | Some(BinaryOp::LogicOp(LogicOp::Or))
    ) as u32
}

/// Union-find connected components on the method graph. Edge
/// shapes: shared self-field OR shared self-method call (either
/// direction). Returns the component count.
fn connected_components(methods: &[MethodInfo]) -> u32 {
    let mut parent: Vec<usize> = (0..methods.len()).collect();
    for i in 0..methods.len() {
        for j in (i + 1)..methods.len() {
            if shares_state(&methods[i], &methods[j]) {
                union(&mut parent, i, j);
            }
        }
    }
    let mut roots: HashSet<usize> = HashSet::new();
    for i in 0..methods.len() {
        roots.insert(find_root(&mut parent, i));
    }
    roots.len() as u32
}

fn shares_state(a: &MethodInfo, b: &MethodInfo) -> bool {
    if a.fields.intersection(&b.fields).next().is_some() {
        return true;
    }
    if let Some(a_fn) = a.hir_fn {
        if b.self_callees.contains(&a_fn) {
            return true;
        }
    }
    if let Some(b_fn) = b.hir_fn {
        if a.self_callees.contains(&b_fn) {
            return true;
        }
    }
    false
}

fn find_root(parent: &mut [usize], i: usize) -> usize {
    if parent[i] != i {
        parent[i] = find_root(parent, parent[i]);
    }
    parent[i]
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let ra = find_root(parent, a);
    let rb = find_root(parent, b);
    if ra != rb {
        parent[ra] = rb;
    }
}

/// `RFC = |M ∪ R|`. Folds every method's `rfc_callees` set into a
/// single global R, then unions M.
fn compute_rfc(methods: &[MethodInfo]) -> u32 {
    let mut r: HashSet<ra_ap_hir::Function> = HashSet::new();
    for m in methods {
        for callee in &m.rfc_callees {
            r.insert(*callee);
        }
    }
    let mut m_set: HashSet<&str> = HashSet::new();
    for m in methods {
        m_set.insert(m.name.as_str());
    }
    // Remove any R entry whose name matches an M name (avoid
    // double-counting methods that are both defined in M and
    // recursively called from M).
    let mut r_names: HashSet<String> = HashSet::new();
    for callee in &r {
        if methods.iter().any(|mi| mi.hir_fn == Some(*callee)) {
            // This callee is in M; don't add to R.
            continue;
        }
        // Use callee identity (Function) as the dedup key; the
        // count is the resulting set's size.
        r_names.insert(format!("{callee:?}"));
    }
    (m_set.len() + r_names.len()) as u32
}

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
        std::env::temp_dir().join(format!("rustics-ra-cohesion-{label}-{pid}-{n}-{s}"))
    }

    fn write_fixture(dir: &std::path::Path, lib_rs: &str) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"cohesion\"\nversion = \"0.0.1\"\nedition = \"2021\"\npublish = false\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .unwrap();
        fs::write(dir.join("src/lib.rs"), lib_rs).unwrap();
    }

    fn find<'a>(out: &'a [ImplCohesion], scope: &str) -> &'a ImplCohesion {
        out.iter()
            .find(|m| m.scope == scope)
            .unwrap_or_else(|| panic!("no measurement for {scope}; got: {out:?}"))
    }

    #[test]
    fn aliased_self_field_access_unifies_methods() {
        // `a` uses `self.x`; `b` aliases `self` then accesses the
        // same field. The AST walker would only credit the bare
        // `self` form and report LCOM4 = 2. HIR sees the same
        // field-share edge through type resolution → LCOM4 = 1.
        let dir = unique_dir("aliased-self");
        write_fixture(
            &dir,
            "pub struct S { pub x: i32 }\n\
             impl S {\n    \
                pub fn a(&self) -> i32 { self.x }\n    \
                pub fn b(&self) -> i32 { let s = self; s.x }\n\
             }\n",
        );
        let out = detect_at(&dir).expect("detect_at");
        let m = find(&out, "S");
        assert_eq!(m.lcom4, Some(1), "aliased self should unify: {m:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn qualified_self_call_unifies_methods() {
        // `a` calls `Self::c`; `b` calls `c` via method syntax.
        // Both should land on the same `c` callee handle through
        // HIR — LCOM4 = 1.
        let dir = unique_dir("qualified-self");
        write_fixture(
            &dir,
            "pub struct S;\n\
             impl S {\n    \
                pub fn a(&self) -> i32 { Self::c(self) }\n    \
                pub fn b(&self) -> i32 { self.c() }\n    \
                pub fn c(&self) -> i32 { 0 }\n\
             }\n",
        );
        let out = detect_at(&dir).expect("detect_at");
        let m = find(&out, "S");
        assert_eq!(m.lcom4, Some(1), "qualified self call unifies: {m:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rfc_excludes_free_function_calls() {
        // The AST RFC walker counts both `module::helper()` (free
        // fn) and `Type::method()` (associated fn). HIR resolves
        // each and only the associated fn lands in R. With one
        // method `m` and one free-fn call inside it, RFC = 1.
        let dir = unique_dir("rfc-free");
        write_fixture(
            &dir,
            "pub mod helpers { pub fn helper() -> i32 { 0 } }\n\
             pub struct S;\n\
             impl S {\n    \
                pub fn m(&self) -> i32 { helpers::helper() }\n\
             }\n",
        );
        let out = detect_at(&dir).expect("detect_at");
        let m = find(&out, "S");
        assert_eq!(m.rfc, 1, "free-fn call must not enter R: {m:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}
