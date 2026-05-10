//! `cargo rustics unused --apply` — orphan deletion.
//!
//! For every [`UnusedItem`] the detector emits, look the declaration
//! up in its source file's CST, take the byte range that covers the
//! item *plus its leading attributes / doc-comments* (ra_ap_syntax
//! includes both as children of the item node, so the node's
//! `text_range()` already covers them), and splice the range out.
//! Multiple deletions in the same file run in descending byte-offset
//! order so each splice doesn't shift the offsets of the remaining
//! ones.
//!
//! Per-kind range strategy:
//!
//! * **Top-level** (`fn` / `struct` / `enum` / `trait` / `type` /
//!   `const` / `static` / `union`) — the item's `text_range`, plus a
//!   trailing newline so the file doesn't keep an empty line where
//!   the item used to live.
//! * **Method** (inherent `impl Foo { fn m() {} }`) and
//!   **assoc-const** (`impl Foo { const K: u8 = 1; }`) — same
//!   strategy applied to the `AssocItem`. Methods and consts inside
//!   an inherent `impl` aren't comma-separated, so no sibling-aware
//!   splicing is needed.
//! * **Variant** (`enum E { A, B, C }`) — comma-aware. Middle / first
//!   position takes `[start(self) .. start(next))`, swallowing the
//!   trailing comma after `self`. Last position takes
//!   `[prev.end .. self.end)`, swallowing the comma between `prev`
//!   and `self`. Removing the only variant in an enum returns
//!   [`LocatorResult::Unsupported`] — the resulting `enum E {}` is
//!   technically valid Rust but represents an uninhabited type the
//!   user almost certainly doesn't intend; let them remove the whole
//!   enum if that's what they want.
//!
//! Safety rails before any edit lands:
//!
//! * `git_tree_is_clean` returns `false` if `git status --porcelain`
//!   prints anything; the CLI refuses to apply unless `--force` is
//!   set.
//! * Files under `tests/` or `**/integration_test/**` are skipped
//!   unless `--include-tests` is set; orphan helpers there are
//!   usually intentional.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use ra_ap_syntax::{
    ast::{self, AstNode, HasModuleItem, HasName},
    Edition, SourceFile, SyntaxNode,
};

use super::UnusedItem;

/// Tally of what `apply` did during one invocation.
#[derive(Debug, Default, Clone, Copy)]
pub struct Outcome {
    /// Declarations whose source range was deleted from disk.
    pub deleted: usize,
    /// Number of distinct files mutated.
    pub touched_files: usize,
    /// Items that lived under `tests/` or `**/integration_test/**`
    /// and were skipped because `--include-tests` was not set.
    pub skipped_test_files: usize,
    /// Items the locator declined to remove because doing so would
    /// leave invalid Rust (e.g. dropping the only variant of an
    /// `enum`). The user gets a notice; the file is untouched for
    /// that entry.
    pub skipped_unsupported: usize,
    /// Items the locator could not find in the freshly-parsed
    /// source — typically because the file changed between detect
    /// and apply, or a re-export shifted the line.
    pub not_found: usize,
}

/// Returns `true` when `git status --porcelain` prints nothing —
/// i.e. the working tree has no uncommitted, untracked, or staged
/// changes inside the workspace root. Returns `Ok(true)` if `git`
/// is unavailable or the workspace isn't a git repo, so users
/// driving the tool outside git aren't blocked from `--apply`.
pub fn git_tree_is_clean(workspace_root: &Path) -> Result<bool> {
    let output = match Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .arg("status")
        .arg("--porcelain")
        .output()
    {
        Ok(o) => o,
        Err(_) => return Ok(true), // git not available → treat as clean
    };
    if !output.status.success() {
        return Ok(true);
    }
    Ok(output.stdout.is_empty())
}

/// Top-level entry point. Filters `items` by file path, groups by
/// owning file, computes a deletion range for each candidate, and
/// writes the splices back.
pub fn apply(workspace_root: &Path, items: &[UnusedItem], include_tests: bool) -> Result<Outcome> {
    let mut outcome = Outcome::default();
    let by_file = partition_by_file(items, include_tests, &mut outcome, workspace_root);
    for (abs, file_items) in by_file {
        let mutated = apply_file(&abs, &file_items, &mut outcome)?;
        if mutated > 0 {
            outcome.touched_files += 1;
        }
    }
    Ok(outcome)
}

/// Splits `items` by file, accumulating `skipped_test_files`
/// counts upfront for entries the path filter excludes.
fn partition_by_file<'a>(
    items: &'a [UnusedItem],
    include_tests: bool,
    outcome: &mut Outcome,
    workspace_root: &Path,
) -> HashMap<PathBuf, Vec<&'a UnusedItem>> {
    let mut by_file: HashMap<PathBuf, Vec<&UnusedItem>> = HashMap::new();
    for item in items {
        if !include_tests && is_test_path(&item.file) {
            outcome.skipped_test_files += 1;
            continue;
        }
        let abs = workspace_root.join(&item.file);
        by_file.entry(abs).or_default().push(item);
    }
    by_file
}

/// Path filter: every path under `tests/` (workspace-level
/// integration tests + per-crate integration tests) and any path
/// with a path component `integration_test` is treated as test
/// scaffolding.
pub(super) fn is_test_path(rel: &str) -> bool {
    rel.starts_with("tests/")
        || rel.contains("/tests/")
        || rel.contains("/integration_test/")
        || rel.starts_with("integration_test/")
}

/// Reads `path`, parses it, looks each requested item up in the
/// CST, and rewrites the file with those byte ranges spliced out.
/// Returns the number of items actually deleted; updates `outcome`
/// in place to record skips that don't show up as deletions.
fn apply_file(path: &Path, items: &[&UnusedItem], outcome: &mut Outcome) -> Result<usize> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("read {} for --apply", path.display()))?;
    let parsed = SourceFile::parse(&source, Edition::CURRENT);
    let tree = parsed.tree();

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for item in items {
        match find_item_range(&tree, &source, item) {
            LocatorResult::Resolved(range) => {
                ranges.push(range);
                outcome.deleted += 1;
            }
            LocatorResult::Unsupported => outcome.skipped_unsupported += 1,
            LocatorResult::NotFound => outcome.not_found += 1,
        }
    }
    if ranges.is_empty() {
        return Ok(0);
    }
    write_splices(path, source, ranges)
}

fn write_splices(path: &Path, source: String, ranges: Vec<(usize, usize)>) -> Result<usize> {
    // Coalesce overlapping / nested ranges first — when the user
    // requests deleting an enum *and* a variant inside it, the
    // variant's range is subsumed by the enum's, and applying the
    // pair without merging would walk past the (already-shrunk)
    // source on the second splice.
    let original_count = ranges.len();
    let merged = merge_ranges(ranges);
    // Sort descending by start byte so each splice doesn't shift
    // the indices of later splices.
    let mut ordered = merged;
    ordered.sort_by(|a, b| b.0.cmp(&a.0));
    let mut new_source = source;
    for (start, end) in &ordered {
        new_source.replace_range(start..end, "");
    }
    std::fs::write(path, &new_source)
        .with_context(|| format!("write {} after --apply", path.display()))?;
    // Each input target still counts as deleted — the merge is an
    // implementation detail; the user asked to remove N items and
    // the source no longer contains them.
    Ok(original_count)
}

/// Coalesces overlapping or nested byte ranges. After merging the
/// returned list is non-overlapping in source order — an outer
/// range subsumes every inner range it contains, so applying the
/// merged list once produces the desired "everything inside the
/// outer slice goes" shape without each `replace_range` having to
/// know about its siblings.
fn merge_ranges(input: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    if input.is_empty() {
        return input;
    }
    let mut sorted = input;
    sorted.sort_by_key(|r| r.0);
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(sorted.len());
    let mut current = sorted[0];
    for next in sorted.into_iter().skip(1) {
        if next.0 < current.1 {
            if next.1 > current.1 {
                current.1 = next.1;
            }
        } else {
            out.push(current);
            current = next;
        }
    }
    out.push(current);
    out
}

/// Outcome of one declaration's locator pass.
enum LocatorResult {
    /// Range to splice out, in source bytes.
    Resolved((usize, usize)),
    /// Locator declined to remove (would leave invalid Rust).
    Unsupported,
    /// Item the detector surfaced doesn't exist in the freshly-
    /// parsed source any more (drift between detect and apply).
    NotFound,
}

/// Per-kind dispatch.
fn find_item_range(file: &SourceFile, source: &str, item: &UnusedItem) -> LocatorResult {
    match item.kind {
        "fn" | "struct" | "enum" | "trait" | "type" | "const" | "static" | "union" => {
            find_top_level(file, source, &item.name, item.kind)
        }
        "method" => find_method(file, source, item.parent.as_deref(), &item.name, item.line),
        "variant" => find_variant(file, source, item.parent.as_deref(), &item.name, item.line),
        "assoc-const" => {
            find_assoc_const(file, source, item.parent.as_deref(), &item.name, item.line)
        }
        _ => LocatorResult::NotFound,
    }
}

fn find_top_level(file: &SourceFile, source: &str, name: &str, kind: &str) -> LocatorResult {
    find_top_level_in(file.items(), source, name, kind)
}

fn find_top_level_in(
    items: impl Iterator<Item = ast::Item>,
    source: &str,
    name: &str,
    kind: &str,
) -> LocatorResult {
    for item in items {
        if matches_top_level(&item, name, kind) {
            return LocatorResult::Resolved(item_byte_range(item.syntax(), source));
        }
        if let ast::Item::Module(m) = &item {
            if let Some(list) = m.item_list() {
                let nested = find_top_level_in(list.items(), source, name, kind);
                if !matches!(nested, LocatorResult::NotFound) {
                    return nested;
                }
            }
        }
    }
    LocatorResult::NotFound
}

fn matches_top_level(item: &ast::Item, name: &str, kind: &str) -> bool {
    let item_name = match item {
        ast::Item::Fn(i) if kind == "fn" => i.name(),
        ast::Item::Struct(i) if kind == "struct" => i.name(),
        ast::Item::Enum(i) if kind == "enum" => i.name(),
        ast::Item::Trait(i) if kind == "trait" => i.name(),
        ast::Item::TypeAlias(i) if kind == "type" => i.name(),
        ast::Item::Const(i) if kind == "const" => i.name(),
        ast::Item::Static(i) if kind == "static" => i.name(),
        ast::Item::Union(i) if kind == "union" => i.name(),
        _ => return false,
    };
    item_name.is_some_and(|n| n.text() == name)
}

fn find_method(
    file: &SourceFile,
    source: &str,
    parent: Option<&str>,
    name: &str,
    line: usize,
) -> LocatorResult {
    let Some(parent_name) = parent else {
        return LocatorResult::NotFound;
    };
    walk_inherent_impls(file.items(), parent_name, |i| {
        find_method_in_impl(i, source, name, line)
    })
}

fn find_method_in_impl(
    i: &ast::Impl,
    source: &str,
    name: &str,
    line: usize,
) -> Option<LocatorResult> {
    let list = i.assoc_item_list()?;
    for ai in list.assoc_items() {
        let ast::AssocItem::Fn(f) = ai else {
            continue;
        };
        let Some(fname) = f.name() else {
            continue;
        };
        if fname.text() == name && line_of(source, fname.syntax()) == line {
            return Some(LocatorResult::Resolved(item_byte_range(f.syntax(), source)));
        }
    }
    None
}

fn find_assoc_const(
    file: &SourceFile,
    source: &str,
    parent: Option<&str>,
    name: &str,
    line: usize,
) -> LocatorResult {
    let Some(parent_name) = parent else {
        return LocatorResult::NotFound;
    };
    walk_inherent_impls(file.items(), parent_name, |i| {
        find_const_in_impl(i, source, name, line)
    })
}

fn find_const_in_impl(
    i: &ast::Impl,
    source: &str,
    name: &str,
    line: usize,
) -> Option<LocatorResult> {
    let list = i.assoc_item_list()?;
    for ai in list.assoc_items() {
        let ast::AssocItem::Const(c) = ai else {
            continue;
        };
        let Some(cname) = c.name() else {
            continue;
        };
        if cname.text() == name && line_of(source, cname.syntax()) == line {
            return Some(LocatorResult::Resolved(item_byte_range(c.syntax(), source)));
        }
    }
    None
}

/// Walks every inherent `impl` block in `items` (recursing through
/// `mod m { ... }`) whose `Self` last-segment matches `parent_name`,
/// invoking `f` until it returns `Some`. Used by both method and
/// assoc-const lookups.
fn walk_inherent_impls<F>(
    items: impl Iterator<Item = ast::Item>,
    parent_name: &str,
    mut f: F,
) -> LocatorResult
where
    F: FnMut(&ast::Impl) -> Option<LocatorResult>,
{
    walk_impls_inner(items, parent_name, &mut f).unwrap_or(LocatorResult::NotFound)
}

fn walk_impls_inner<F>(
    items: impl Iterator<Item = ast::Item>,
    parent_name: &str,
    f: &mut F,
) -> Option<LocatorResult>
where
    F: FnMut(&ast::Impl) -> Option<LocatorResult>,
{
    for item in items {
        if let Some(hit) = visit_for_inherent_impl(item, parent_name, f) {
            return Some(hit);
        }
    }
    None
}

fn visit_for_inherent_impl<F>(
    item: ast::Item,
    parent_name: &str,
    f: &mut F,
) -> Option<LocatorResult>
where
    F: FnMut(&ast::Impl) -> Option<LocatorResult>,
{
    match item {
        ast::Item::Impl(i) if is_matching_inherent_impl(&i, parent_name) => f(&i),
        ast::Item::Module(m) => m
            .item_list()
            .and_then(|list| walk_impls_inner(list.items(), parent_name, f)),
        _ => None,
    }
}

fn is_matching_inherent_impl(i: &ast::Impl, parent_name: &str) -> bool {
    i.trait_().is_none()
        && i.self_ty()
            .as_ref()
            .and_then(type_path_last_segment)
            .as_deref()
            == Some(parent_name)
}

fn find_variant(
    file: &SourceFile,
    source: &str,
    parent: Option<&str>,
    name: &str,
    line: usize,
) -> LocatorResult {
    let Some(parent_name) = parent else {
        return LocatorResult::NotFound;
    };
    find_variant_in_items(file.items(), source, parent_name, name, line)
}

fn find_variant_in_items(
    items: impl Iterator<Item = ast::Item>,
    source: &str,
    parent_name: &str,
    name: &str,
    line: usize,
) -> LocatorResult {
    for item in items {
        match item {
            ast::Item::Enum(e) if e.name().is_some_and(|n| n.text() == parent_name) => {
                let result = locate_variant_in_enum(&e, source, name, line);
                if !matches!(result, LocatorResult::NotFound) {
                    return result;
                }
            }
            ast::Item::Module(m) => {
                if let Some(list) = m.item_list() {
                    let nested =
                        find_variant_in_items(list.items(), source, parent_name, name, line);
                    if !matches!(nested, LocatorResult::NotFound) {
                        return nested;
                    }
                }
            }
            _ => {}
        }
    }
    LocatorResult::NotFound
}

fn locate_variant_in_enum(e: &ast::Enum, source: &str, name: &str, line: usize) -> LocatorResult {
    let Some(list) = e.variant_list() else {
        return LocatorResult::NotFound;
    };
    let variants: Vec<ast::Variant> = list.variants().collect();
    let Some(idx) = variants.iter().position(|v| {
        v.name()
            .is_some_and(|n| n.text() == name && line_of(source, n.syntax()) == line)
    }) else {
        return LocatorResult::NotFound;
    };
    if variants.len() == 1 {
        // Removing the last variant would leave `enum E {}` — valid
        // Rust but uninhabited. Refuse and let the user decide.
        return LocatorResult::Unsupported;
    }
    LocatorResult::Resolved(variant_byte_range(&variants, idx, source))
}

/// Comma-aware splice for one variant inside `variants`:
///
/// * middle / first (`idx < len - 1`): from this variant's start to
///   the next variant's start. Swallows the comma after `self` and
///   any whitespace before `next`.
/// * last (`idx == len - 1`): from the previous variant's end to
///   this variant's end — plus the *optional trailing comma*
///   (`enum E { A, B, C, }`). Skipping the trailing comma would
///   leave a stray `,` when every variant of an enum is deleted in
///   one run.
fn variant_byte_range(variants: &[ast::Variant], idx: usize, source: &str) -> (usize, usize) {
    let target = &variants[idx];
    let target_range = target.syntax().text_range();
    if idx + 1 < variants.len() {
        let next = &variants[idx + 1];
        return (
            target_range.start().into(),
            next.syntax().text_range().start().into(),
        );
    }
    let prev = &variants[idx - 1];
    let start: usize = prev.syntax().text_range().end().into();
    let end_after_target: usize = target_range.end().into();
    let end = extend_through_immediate_comma(end_after_target, source);
    (start, end)
}

/// If `end` lands directly on a `,` byte, consume it. Used by the
/// last-variant range computation to swallow the optional trailing
/// comma after the last `enum` variant. Whitespace is *not* skipped
/// — `enum E { A, B, C , }` (variant followed by space, comma) is
/// vanishingly rare and over-eating whitespace would risk pulling
/// in unrelated tokens on edge-case inputs.
fn extend_through_immediate_comma(end: usize, source: &str) -> usize {
    if source.as_bytes().get(end) == Some(&b',') {
        end + 1
    } else {
        end
    }
}

/// Computes the byte range that covers `node` in `source`, plus a
/// trailing newline so a clean splice doesn't leave an orphan blank
/// line. ra_ap_syntax includes leading attributes and doc comments
/// as children of the item node, so the node's `text_range()` already
/// covers them — no extra "earliest of {item start, attrs start}"
/// computation is needed (unlike the syn version, where attrs and the
/// item's `Span` were separate).
fn item_byte_range(node: &SyntaxNode, source: &str) -> (usize, usize) {
    let range = node.text_range();
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    let end = consume_trailing_newline(end, source);
    (start, end)
}

fn consume_trailing_newline(end: usize, source: &str) -> usize {
    if source.as_bytes().get(end) == Some(&b'\n') {
        end + 1
    } else {
        end
    }
}

/// Last segment of a path-typed self type. `impl Foo<T>` → `Foo`.
/// PathType / RefType / ParenType are the receiver shapes inherent
/// impls actually take; everything else (tuples, fn-pointers) gives
/// `None` and is ignored by the caller.
fn type_path_last_segment(ty: &ast::Type) -> Option<String> {
    match ty {
        ast::Type::PathType(p) => p
            .path()
            .and_then(|p| p.segment())
            .and_then(|s| s.name_ref())
            .map(|n| n.text().to_string()),
        ast::Type::RefType(r) => r.ty().as_ref().and_then(type_path_last_segment),
        ast::Type::ParenType(p) => p.ty().as_ref().and_then(type_path_last_segment),
        _ => None,
    }
}

/// 1-based line of `node`'s starting byte. Mirrors the helper in
/// `super::line_of` and `cross_file::trait_impl_fanout::line_of`;
/// the three callers each compute lines from a different shape
/// (top-level node, ident node, impl node) so a single shared helper
/// would only paper over the call-site differences.
fn line_of(source: &str, node: &SyntaxNode) -> usize {
    let offset: usize = node.text_range().start().into();
    source
        .get(..offset)
        .unwrap_or("")
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn tempdir(label: &str) -> PathBuf {
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rustics-apply-{label}-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
        let abs = dir.join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, body).unwrap();
        abs
    }

    /// Test helper: parses `src` and returns Err when the parser had
    /// any structural diagnostics. Used to assert that post-splice
    /// source is still valid Rust.
    fn parse_ok(src: &str) {
        let parsed = SourceFile::parse(src, Edition::CURRENT);
        let errors = parsed.errors();
        assert!(
            errors.is_empty(),
            "post-splice source has parse errors: {errors:?}\nsource:\n{src}"
        );
    }

    fn item(file: &str, name: &str, kind: &'static str) -> UnusedItem {
        item_with(file, name, kind, None, 1)
    }

    fn item_with(
        file: &str,
        name: &str,
        kind: &'static str,
        parent: Option<&str>,
        line: usize,
    ) -> UnusedItem {
        UnusedItem {
            file: file.to_string(),
            line,
            name: name.to_string(),
            kind,
            parent: parent.map(str::to_string),
        }
    }

    #[test]
    fn is_test_path_recognises_workspace_and_crate_tests() {
        assert!(is_test_path("tests/foo.rs"));
        assert!(is_test_path("crates/foo/tests/bar.rs"));
        assert!(is_test_path("crates/foo/integration_test/baz.rs"));
        assert!(is_test_path("integration_test/qux.rs"));
        assert!(!is_test_path("crates/foo/src/lib.rs"));
        assert!(!is_test_path("src/main.rs"));
    }

    #[test]
    fn apply_deletes_top_level_fn() {
        let dir = tempdir("delete-fn");
        let abs = write(
            &dir,
            "src/lib.rs",
            "pub fn keep() {}\npub fn drop_me() {\n    println!(\"x\");\n}\npub fn other() {}\n",
        );
        let outcome = apply(&dir, &[item("src/lib.rs", "drop_me", "fn")], false).unwrap();
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.touched_files, 1);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains("drop_me"), "after = {after:?}");
        assert!(after.contains("keep"));
        assert!(after.contains("other"));
        // The post-splice source must still parse as Rust — catches
        // stray `pub ` left over when the visibility token wasn't
        // covered by the splice.
        parse_ok(&after);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_deletes_attributes_with_item() {
        let dir = tempdir("delete-attrs");
        let abs = write(
            &dir,
            "src/lib.rs",
            "/// Docstring.\n#[deprecated]\npub fn drop_me() {}\npub fn keep() {}\n",
        );
        apply(&dir, &[item("src/lib.rs", "drop_me", "fn")], false).unwrap();
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains("drop_me"));
        assert!(!after.contains("Docstring"));
        assert!(!after.contains("deprecated"));
        assert!(after.contains("keep"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_skips_test_paths_by_default() {
        let dir = tempdir("skip-tests");
        let abs = write(&dir, "tests/helpers.rs", "pub fn helper() {}\n");
        let outcome = apply(&dir, &[item("tests/helpers.rs", "helper", "fn")], false).unwrap();
        assert_eq!(outcome.deleted, 0);
        assert_eq!(outcome.skipped_test_files, 1);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(after.contains("helper"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_can_include_test_paths() {
        let dir = tempdir("incl-tests");
        let abs = write(&dir, "tests/helpers.rs", "pub fn helper() {}\n");
        let outcome = apply(&dir, &[item("tests/helpers.rs", "helper", "fn")], true).unwrap();
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.skipped_test_files, 0);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains("helper"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_descending_order_keeps_offsets_valid() {
        let dir = tempdir("two-in-file");
        let abs = write(
            &dir,
            "src/lib.rs",
            "pub fn one() {}\npub fn two() {}\npub fn three() {}\n",
        );
        let outcome = apply(
            &dir,
            &[
                item("src/lib.rs", "one", "fn"),
                item("src/lib.rs", "three", "fn"),
            ],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 2);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains("one"));
        assert!(!after.contains("three"));
        assert!(after.contains("two"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_unknown_name_is_recorded_as_not_found() {
        let dir = tempdir("ghost-name");
        let _abs = write(&dir, "src/lib.rs", "pub fn alive() {}\n");
        let outcome = apply(&dir, &[item("src/lib.rs", "ghost", "fn")], false).unwrap();
        assert_eq!(outcome.deleted, 0);
        assert_eq!(outcome.not_found, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn git_tree_is_clean_returns_true_outside_git_repo() {
        let dir = tempdir("not-git");
        assert!(git_tree_is_clean(&dir).unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    // -----------------------------------------------------------------
    // Method / variant / assoc-const deletion.
    // -----------------------------------------------------------------

    #[test]
    fn apply_deletes_inherent_method() {
        let dir = tempdir("delete-method");
        // Source line numbers (1-indexed):
        //   1: pub struct Foo;
        //   2: impl Foo {
        //   3:     pub fn keep(&self) {}
        //   4:     pub fn drop_me(&self) {
        //   5:         println!("x");
        //   6:     }
        //   7: }
        let abs = write(
            &dir,
            "src/lib.rs",
            "pub struct Foo;\nimpl Foo {\n    pub fn keep(&self) {}\n    pub fn drop_me(&self) {\n        println!(\"x\");\n    }\n}\n",
        );
        let outcome = apply(
            &dir,
            &[item_with("src/lib.rs", "drop_me", "method", Some("Foo"), 4)],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 1);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains("drop_me"));
        assert!(after.contains("keep"));
        assert!(after.contains("impl Foo"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_deletes_assoc_const() {
        let dir = tempdir("delete-assoc-const");
        let abs = write(
            &dir,
            "src/lib.rs",
            "pub struct Foo;\nimpl Foo {\n    pub const KEEP: u8 = 1;\n    pub const DROP_ME: u8 = 2;\n}\n",
        );
        let outcome = apply(
            &dir,
            &[item_with(
                "src/lib.rs",
                "DROP_ME",
                "assoc-const",
                Some("Foo"),
                4,
            )],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 1);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains("DROP_ME"));
        assert!(after.contains("KEEP"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_deletes_middle_enum_variant_with_comma() {
        let dir = tempdir("delete-variant-mid");
        // Lines:
        //   1: pub enum E {
        //   2:     A,
        //   3:     B,
        //   4:     C,
        //   5: }
        let abs = write(
            &dir,
            "src/lib.rs",
            "pub enum E {\n    A,\n    B,\n    C,\n}\n",
        );
        let outcome = apply(
            &dir,
            &[item_with("src/lib.rs", "B", "variant", Some("E"), 3)],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 1);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains(" B,"));
        assert!(after.contains(" A,"));
        assert!(after.contains(" C,"));
        // Result must still parse as valid Rust.
        parse_ok(&after);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_deletes_first_enum_variant() {
        let dir = tempdir("delete-variant-first");
        let abs = write(
            &dir,
            "src/lib.rs",
            "pub enum E {\n    A,\n    B,\n    C,\n}\n",
        );
        let outcome = apply(
            &dir,
            &[item_with("src/lib.rs", "A", "variant", Some("E"), 2)],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 1);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains(" A,"));
        assert!(after.contains(" B,"));
        parse_ok(&after);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_deletes_last_enum_variant() {
        let dir = tempdir("delete-variant-last");
        // No trailing comma after the last variant on purpose — this
        // is the syntactic case that triggers the `prev.end → self.end`
        // branch.
        let abs = write(
            &dir,
            "src/lib.rs",
            "pub enum E {\n    A,\n    B,\n    C\n}\n",
        );
        let outcome = apply(
            &dir,
            &[item_with("src/lib.rs", "C", "variant", Some("E"), 4)],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 1);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains(" C"));
        assert!(after.contains(" A,"));
        assert!(after.contains(" B"));
        parse_ok(&after);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_refuses_only_variant_in_enum() {
        let dir = tempdir("only-variant");
        let abs = write(&dir, "src/lib.rs", "pub enum E {\n    Solo,\n}\n");
        let outcome = apply(
            &dir,
            &[item_with("src/lib.rs", "Solo", "variant", Some("E"), 2)],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 0);
        assert_eq!(outcome.skipped_unsupported, 1);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(after.contains("Solo"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_method_without_parent_is_not_found() {
        // UnusedItem.parent = None for a method shouldn't crash; it
        // just reports as not found. (Detector populates parent for
        // every method, so this is an invariant test.)
        let dir = tempdir("method-no-parent");
        let _abs = write(
            &dir,
            "src/lib.rs",
            "pub struct Foo; impl Foo { pub fn m() {} }\n",
        );
        let outcome = apply(
            &dir,
            &[item_with("src/lib.rs", "m", "method", None, 1)],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 0);
        assert_eq!(outcome.not_found, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_ranges_coalesces_nested() {
        // [10, 20) contains [12, 18) — merged result is [10, 20).
        let merged = merge_ranges(vec![(10, 20), (12, 18)]);
        assert_eq!(merged, [(10, 20)]);
    }

    #[test]
    fn merge_ranges_coalesces_overlapping() {
        // [10, 15) overlaps [12, 18) → [10, 18).
        let merged = merge_ranges(vec![(10, 15), (12, 18)]);
        assert_eq!(merged, [(10, 18)]);
    }

    #[test]
    fn merge_ranges_keeps_disjoint_separate() {
        let merged = merge_ranges(vec![(0, 5), (10, 15), (20, 25)]);
        assert_eq!(merged, [(0, 5), (10, 15), (20, 25)]);
    }

    #[test]
    fn merge_ranges_handles_empty_input() {
        let merged: Vec<(usize, usize)> = merge_ranges(vec![]);
        assert!(merged.is_empty());
    }

    #[test]
    fn merge_ranges_sorts_unsorted_input() {
        // Caller may push ranges in any order; merge must sort
        // first or it'll skip overlaps it doesn't see in sequence.
        let merged = merge_ranges(vec![(20, 25), (10, 22), (0, 5)]);
        assert_eq!(merged, [(0, 5), (10, 25)]);
    }

    #[test]
    fn apply_deletes_all_variants_with_trailing_comma() {
        // Regression: deleting every variant of `enum E { A, B, C, }`
        // (note the trailing comma after C) used to leave a stray
        // `,` because the last-position range stopped at C's span
        // end and didn't consume the trailing comma. Verify the
        // resulting enum body is empty and the file still parses.
        let dir = tempdir("all-variants-trailing");
        let abs = write(
            &dir,
            "src/lib.rs",
            "pub enum E {\n    A,\n    B,\n    C,\n}\n",
        );
        let outcome = apply(
            &dir,
            &[
                item_with("src/lib.rs", "A", "variant", Some("E"), 2),
                item_with("src/lib.rs", "B", "variant", Some("E"), 3),
                item_with("src/lib.rs", "C", "variant", Some("E"), 4),
            ],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 3);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains(" A,"));
        assert!(!after.contains(" B,"));
        assert!(!after.contains(" C,"));
        // The crucial assertion: no stray comma left between { and }.
        // Source must still parse as Rust.
        parse_ok(&after);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extend_through_immediate_comma_consumes_only_when_present() {
        let src = "C,";
        assert_eq!(extend_through_immediate_comma(1, src), 2);
        let src = "C\n";
        assert_eq!(extend_through_immediate_comma(1, src), 1);
        let src = "C";
        assert_eq!(extend_through_immediate_comma(1, src), 1);
    }

    #[test]
    fn apply_handles_enum_and_variant_in_one_shot() {
        // Regression for the overlap bug: deleting `enum E` *and*
        // `variant E::B` in the same run must not corrupt the
        // post-splice source. The variant's range lives inside the
        // enum's range; merge_ranges coalesces the pair so we
        // splice once.
        let dir = tempdir("enum-and-variant");
        let abs = write(
            &dir,
            "src/lib.rs",
            "pub fn keep() {}\npub enum E {\n    A,\n    B,\n    C,\n}\npub fn other() {}\n",
        );
        let outcome = apply(
            &dir,
            &[
                item_with("src/lib.rs", "B", "variant", Some("E"), 4),
                item_with("src/lib.rs", "E", "enum", None, 2),
            ],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 2);
        let after = std::fs::read_to_string(&abs).unwrap();
        // `keep` and `other` survive; `enum E` and its variants are gone.
        assert!(after.contains("pub fn keep"));
        assert!(after.contains("pub fn other"));
        assert!(!after.contains("enum E"));
        assert!(!after.contains(" A,"));
        assert!(!after.contains(" B,"));
        assert!(!after.contains(" C,"));
        // Result must still parse.
        parse_ok(&after);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_method_in_trait_impl_is_not_found() {
        // Trait impls are deliberately ignored by both the detector
        // and the applier (signature-driven dispatch). A method named
        // `m` only existing in `impl T for Foo` shouldn't match.
        let dir = tempdir("method-trait-impl");
        let _abs = write(
            &dir,
            "src/lib.rs",
            "pub struct Foo; pub trait T { fn m(&self); }\n\
             impl T for Foo { fn m(&self) {} }\n",
        );
        let outcome = apply(
            &dir,
            &[item_with("src/lib.rs", "m", "method", Some("Foo"), 2)],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 0);
        assert_eq!(outcome.not_found, 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
