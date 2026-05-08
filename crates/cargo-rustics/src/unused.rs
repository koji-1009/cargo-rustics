//! Unused public API detection — Periphery-style heuristic.
//!
//! Plan §M3 / §7.1. The detector walks every workspace file, collects
//! every identifier *use* (any `Ident` token), and flags every `pub`
//! item whose name does not appear anywhere outside its declaration.
//!
//! Heuristic, not semantic — a richer check needs name resolution and
//! lands when M3's rust-analyzer integration arrives. The trade-off:
//! the heuristic is fast (single AST pass + token scan) and correct
//! enough to surface obvious dead public items; it does false-positive
//! on items that are only referenced through proc-macro expansion or
//! reflection-style lookups.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use proc_macro2::TokenTree;
use quote::ToTokens;
use syn::{Item, Visibility};

use crate::discover::DiscoveredFile;

/// One unused-public finding.
#[derive(Debug, Clone)]
pub struct UnusedItem {
    /// Workspace-relative path.
    pub file: String,
    /// 1-based line number of the item declaration.
    pub line: usize,
    /// Item name (`fn` / `struct` / etc).
    pub name: String,
    /// Item kind for display.
    pub kind: &'static str,
}

/// Walks `files`, returns every `pub` item whose name is referenced
/// zero times outside its own declaration.
pub fn detect(files: &[DiscoveredFile]) -> Result<Vec<UnusedItem>> {
    let mut declarations: Vec<DeclSite> = Vec::new();
    let mut reference_counts: HashMap<String, u32> = HashMap::new();

    for file in files {
        let source = std::fs::read_to_string(&file.absolute)
            .with_context(|| format!("read {}", file.relative))?;
        let Ok(ast) = syn::parse_file(&source) else {
            continue;
        };
        collect_pub_decls(&file.relative, &ast, &mut declarations);
        count_references(&ast, &mut reference_counts);
    }

    let mut out = Vec::new();
    for decl in &declarations {
        // The declaration itself contributes one reference (the ident
        // token in its definition). An *unused* item has total
        // reference count == 1.
        let count = reference_counts.get(&decl.name).copied().unwrap_or(0);
        if count <= 1 {
            out.push(UnusedItem {
                file: decl.file.clone(),
                line: decl.line,
                name: decl.name.clone(),
                kind: decl.kind,
            });
        }
    }
    out.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.line.cmp(&b.line)));
    Ok(out)
}

#[derive(Debug, Clone)]
struct DeclSite {
    file: String,
    line: usize,
    name: String,
    kind: &'static str,
}

fn collect_pub_decls(file: &str, ast: &syn::File, out: &mut Vec<DeclSite>) {
    for item in &ast.items {
        if let Some(decl) = pub_decl(file, item) {
            out.push(decl);
        }
    }
}

fn pub_decl(file: &str, item: &Item) -> Option<DeclSite> {
    macro_rules! emit {
        ($vis:expr, $ident:expr, $kind:expr) => {{
            if !is_pub(&$vis) {
                return None;
            }
            return Some(DeclSite {
                file: file.to_string(),
                line: $ident.span().start().line,
                name: $ident.to_string(),
                kind: $kind,
            });
        }};
    }
    match item {
        Item::Fn(i) => emit!(i.vis, i.sig.ident, "fn"),
        Item::Struct(i) => emit!(i.vis, i.ident, "struct"),
        Item::Enum(i) => emit!(i.vis, i.ident, "enum"),
        Item::Trait(i) => emit!(i.vis, i.ident, "trait"),
        Item::Type(i) => emit!(i.vis, i.ident, "type"),
        Item::Const(i) => emit!(i.vis, i.ident, "const"),
        Item::Static(i) => emit!(i.vis, i.ident, "static"),
        Item::Union(i) => emit!(i.vis, i.ident, "union"),
        _ => None,
    }
}

fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

fn count_references(ast: &syn::File, counts: &mut HashMap<String, u32>) {
    let stream = ast.to_token_stream();
    walk_tokens(&stream, counts);
}

fn walk_tokens(stream: &proc_macro2::TokenStream, counts: &mut HashMap<String, u32>) {
    for tt in stream.clone() {
        match tt {
            TokenTree::Ident(id) => {
                *counts.entry(id.to_string()).or_insert(0) += 1;
            }
            TokenTree::Group(g) => walk_tokens(&g.stream(), counts),
            _ => {}
        }
    }
}

/// Renders a small reporter-ish text dump for `cargo rustics unused`.
pub fn format(items: &[UnusedItem]) -> String {
    if items.is_empty() {
        return "rustics unused: no candidates found.\n".to_string();
    }
    let mut s = format!("rustics unused: {} candidate(s):\n", items.len());
    for item in items {
        s.push_str(&format!(
            "  {kind} {name} — {file}:{line}\n",
            kind = item.kind,
            name = item.name,
            file = item.file,
            line = item.line,
        ));
    }
    s
}

/// Helper for crate-level workspace lookups (kept here so the binary
/// crate's import graph doesn't grow).
pub fn detect_at(workspace_root: &Path) -> Result<Vec<UnusedItem>> {
    let files = crate::discover::discover_rust_files(
        workspace_root,
        workspace_root,
        &crate::config::ExcludeTable::default(),
    )?;
    detect(&files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_kinds_recognised() {
        let src = "pub fn f() {} pub struct S; pub enum E {} pub trait T {}";
        let ast = syn::parse_file(src).unwrap();
        let mut decls = Vec::new();
        collect_pub_decls("t.rs", &ast, &mut decls);
        let kinds: Vec<&str> = decls.iter().map(|d| d.kind).collect();
        assert!(kinds.contains(&"fn"));
        assert!(kinds.contains(&"struct"));
        assert!(kinds.contains(&"enum"));
        assert!(kinds.contains(&"trait"));
    }

    #[test]
    fn private_items_not_collected() {
        let src = "fn f() {} struct S;";
        let ast = syn::parse_file(src).unwrap();
        let mut decls = Vec::new();
        collect_pub_decls("t.rs", &ast, &mut decls);
        assert!(decls.is_empty());
    }

    #[test]
    fn ident_count_walks_groups() {
        let src = "pub fn f() {} pub fn g() { f(); f(); }";
        let ast = syn::parse_file(src).unwrap();
        let mut counts = HashMap::new();
        count_references(&ast, &mut counts);
        // `f` appears once at decl, twice in g's body -> 3.
        assert!(counts.get("f").copied().unwrap_or(0) >= 3);
    }

    #[test]
    fn format_says_no_candidates_when_empty() {
        assert_eq!(format(&[]), "rustics unused: no candidates found.\n");
    }
}
