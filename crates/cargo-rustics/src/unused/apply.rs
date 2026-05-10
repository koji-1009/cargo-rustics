//! `cargo rustics unused --apply` — orphan deletion.
//!
//! For every [`UnusedItem`] the detector emits, look the declaration
//! up in its source file's AST, compute the byte range that covers
//! the item *plus its leading attributes / doc-comments*, and splice
//! the range out. Multiple deletions in the same file run in
//! descending byte-offset order so each splice doesn't shift the
//! offsets of the remaining ones.
//!
//! Per-kind range strategy:
//!
//! * **Top-level** (`fn` / `struct` / `enum` / `trait` / `type` /
//!   `const` / `static` / `union`) — earliest-leading-attr through
//!   `Item::span().end()`, plus a trailing newline so the file
//!   doesn't keep an empty line where the item used to live.
//! * **Method** (inherent `impl Foo { fn m() {} }`) and
//!   **assoc-const** (`impl Foo { const K: u8 = 1; }`) — same
//!   leading-attr-through-end strategy applied to the `ImplItem`.
//!   Methods and consts inside an inherent `impl` aren't comma-
//!   separated, so no sibling-aware splicing is needed.
//! * **Variant** (`enum E { A, B, C }`) — comma-aware. Middle / first
//!   position takes `[leading_start(self) .. leading_start(next))`,
//!   swallowing the trailing comma after `self`. Last position takes
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
use proc_macro2::LineColumn;
use syn::spanned::Spanned;
use syn::{Attribute, ImplItem, Item, ItemEnum, ItemImpl, Type, Variant, Visibility};

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
/// AST, and rewrites the file with those byte ranges spliced out.
/// Returns the number of items actually deleted; updates `outcome`
/// in place to record skips that don't show up as deletions.
fn apply_file(path: &Path, items: &[&UnusedItem], outcome: &mut Outcome) -> Result<usize> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("read {} for --apply", path.display()))?;
    let ast = syn::parse_file(&source)
        .with_context(|| format!("parse {} for --apply", path.display()))?;
    let line_starts = compute_line_starts(&source);

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for item in items {
        match find_item_range(&ast, &source, &line_starts, item) {
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
    // source on the second splice. Mirrors dartrics's
    // _mergeRanges.
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
fn find_item_range(
    file: &syn::File,
    source: &str,
    line_starts: &[usize],
    item: &UnusedItem,
) -> LocatorResult {
    match item.kind {
        "fn" | "struct" | "enum" | "trait" | "type" | "const" | "static" | "union" => {
            find_top_level(file, source, line_starts, &item.name, item.kind)
        }
        "method" => find_method(
            file,
            source,
            line_starts,
            item.parent.as_deref(),
            &item.name,
            item.line,
        ),
        "variant" => find_variant(
            file,
            source,
            line_starts,
            item.parent.as_deref(),
            &item.name,
            item.line,
        ),
        "assoc-const" => find_assoc_const(
            file,
            source,
            line_starts,
            item.parent.as_deref(),
            &item.name,
            item.line,
        ),
        _ => LocatorResult::NotFound,
    }
}

fn find_top_level(
    file: &syn::File,
    source: &str,
    line_starts: &[usize],
    name: &str,
    kind: &str,
) -> LocatorResult {
    find_top_level_in(&file.items, source, line_starts, name, kind)
}

fn find_top_level_in(
    items: &[Item],
    source: &str,
    line_starts: &[usize],
    name: &str,
    kind: &str,
) -> LocatorResult {
    for item in items {
        if matches_top_level(item, name, kind) {
            return LocatorResult::Resolved(item_byte_range(item, source, line_starts));
        }
        if let Item::Mod(m) = item {
            if let Some((_, inner)) = &m.content {
                let nested = find_top_level_in(inner, source, line_starts, name, kind);
                if !matches!(nested, LocatorResult::NotFound) {
                    return nested;
                }
            }
        }
    }
    LocatorResult::NotFound
}

fn matches_top_level(item: &Item, name: &str, kind: &str) -> bool {
    match (item, kind) {
        (Item::Fn(i), "fn") => i.sig.ident == name,
        (Item::Struct(i), "struct") => i.ident == name,
        (Item::Enum(i), "enum") => i.ident == name,
        (Item::Trait(i), "trait") => i.ident == name,
        (Item::Type(i), "type") => i.ident == name,
        (Item::Const(i), "const") => i.ident == name,
        (Item::Static(i), "static") => i.ident == name,
        (Item::Union(i), "union") => i.ident == name,
        _ => false,
    }
}

fn find_method(
    file: &syn::File,
    source: &str,
    line_starts: &[usize],
    parent: Option<&str>,
    name: &str,
    line: usize,
) -> LocatorResult {
    let Some(parent_name) = parent else {
        return LocatorResult::NotFound;
    };
    walk_inherent_impls(&file.items, parent_name, |i| find_method_in_impl(i, source, line_starts, name, line))
}

fn find_method_in_impl(
    i: &ItemImpl,
    source: &str,
    line_starts: &[usize],
    name: &str,
    line: usize,
) -> Option<LocatorResult> {
    for ii in &i.items {
        if let ImplItem::Fn(f) = ii {
            if f.sig.ident == name && f.sig.ident.span().start().line == line {
                return Some(LocatorResult::Resolved(impl_item_fn_range(
                    f,
                    source,
                    line_starts,
                )));
            }
        }
    }
    None
}

fn find_assoc_const(
    file: &syn::File,
    source: &str,
    line_starts: &[usize],
    parent: Option<&str>,
    name: &str,
    line: usize,
) -> LocatorResult {
    let Some(parent_name) = parent else {
        return LocatorResult::NotFound;
    };
    walk_inherent_impls(&file.items, parent_name, |i| find_const_in_impl(i, source, line_starts, name, line))
}

fn find_const_in_impl(
    i: &ItemImpl,
    source: &str,
    line_starts: &[usize],
    name: &str,
    line: usize,
) -> Option<LocatorResult> {
    for ii in &i.items {
        if let ImplItem::Const(c) = ii {
            if c.ident == name && c.ident.span().start().line == line {
                return Some(LocatorResult::Resolved(impl_item_const_range(
                    c,
                    source,
                    line_starts,
                )));
            }
        }
    }
    None
}

/// Walks every inherent `impl` block in `items` (recursing through
/// `mod m { ... }`) whose `Self` last-segment matches `parent_name`,
/// invoking `f` until it returns `Some`. Used by both method and
/// assoc-const lookups.
fn walk_inherent_impls<F>(items: &[Item], parent_name: &str, mut f: F) -> LocatorResult
where
    F: FnMut(&ItemImpl) -> Option<LocatorResult>,
{
    walk_impls_inner(items, parent_name, &mut f).unwrap_or(LocatorResult::NotFound)
}

fn walk_impls_inner<F>(items: &[Item], parent_name: &str, f: &mut F) -> Option<LocatorResult>
where
    F: FnMut(&ItemImpl) -> Option<LocatorResult>,
{
    for item in items {
        if let Some(hit) = visit_for_inherent_impl(item, parent_name, f) {
            return Some(hit);
        }
    }
    None
}

fn visit_for_inherent_impl<F>(
    item: &Item,
    parent_name: &str,
    f: &mut F,
) -> Option<LocatorResult>
where
    F: FnMut(&ItemImpl) -> Option<LocatorResult>,
{
    match item {
        Item::Impl(i) if is_matching_inherent_impl(i, parent_name) => f(i),
        Item::Mod(m) => m
            .content
            .as_ref()
            .and_then(|(_, inner)| walk_impls_inner(inner, parent_name, f)),
        _ => None,
    }
}

fn is_matching_inherent_impl(i: &ItemImpl, parent_name: &str) -> bool {
    i.trait_.is_none() && type_path_last_segment(&i.self_ty).as_deref() == Some(parent_name)
}

fn find_variant(
    file: &syn::File,
    source: &str,
    line_starts: &[usize],
    parent: Option<&str>,
    name: &str,
    line: usize,
) -> LocatorResult {
    let Some(parent_name) = parent else {
        return LocatorResult::NotFound;
    };
    find_variant_in_items(&file.items, source, line_starts, parent_name, name, line)
}

fn find_variant_in_items(
    items: &[Item],
    source: &str,
    line_starts: &[usize],
    parent_name: &str,
    name: &str,
    line: usize,
) -> LocatorResult {
    for item in items {
        match item {
            Item::Enum(e) if e.ident == parent_name => {
                let result = locate_variant_in_enum(e, source, line_starts, name, line);
                if !matches!(result, LocatorResult::NotFound) {
                    return result;
                }
            }
            Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    let nested =
                        find_variant_in_items(inner, source, line_starts, parent_name, name, line);
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

fn locate_variant_in_enum(
    e: &ItemEnum,
    source: &str,
    line_starts: &[usize],
    name: &str,
    line: usize,
) -> LocatorResult {
    let count = e.variants.len();
    let Some(idx) = e
        .variants
        .iter()
        .position(|v| v.ident == name && v.ident.span().start().line == line)
    else {
        return LocatorResult::NotFound;
    };
    if count == 1 {
        // Removing the last variant would leave `enum E {}` — valid
        // Rust but uninhabited. Refuse and let the user decide.
        return LocatorResult::Unsupported;
    }
    LocatorResult::Resolved(variant_byte_range(&e.variants, idx, source, line_starts))
}

/// Comma-aware splice for one variant inside `variants`:
///
/// * middle / first (`idx < len - 1`): from this variant's leading
///   start to the next variant's leading start. Swallows the comma
///   after `self` and any whitespace before `next`.
/// * last (`idx == len - 1`): from the previous variant's `.end()`
///   to this variant's `.end()` — plus the *optional trailing
///   comma* after the last variant (`enum E { A, B, C, }`). dartrics
///   does without that step because Dart's analyzer surfaces it as
///   tolerated trailing punctuation, but skipping it here would
///   leave a stray `,` when every variant of an enum is deleted in
///   one run (the merged range would stop right before the trailing
///   comma).
fn variant_byte_range(
    variants: &syn::punctuated::Punctuated<Variant, syn::Token![,]>,
    idx: usize,
    source: &str,
    line_starts: &[usize],
) -> (usize, usize) {
    let target = &variants[idx];
    if idx + 1 < variants.len() {
        let next = &variants[idx + 1];
        return (
            leading_start(&target.attrs, target.span().start(), source, line_starts),
            leading_start(&next.attrs, next.span().start(), source, line_starts),
        );
    }
    let prev = &variants[idx - 1];
    let start = line_col_to_byte(source, line_starts, prev.span().end());
    let end_after_target = line_col_to_byte(source, line_starts, target.span().end());
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

/// Computes the byte range that covers `item` *and its leading
/// attributes* in `source`, plus a trailing newline so a clean
/// splice doesn't leave an orphan blank line.
fn item_byte_range(item: &Item, source: &str, line_starts: &[usize]) -> (usize, usize) {
    let attrs = item_attrs(item);
    let main_start = item.span().start();
    let start = leading_start(attrs, main_start, source, line_starts);
    let end_lc = item.span().end();
    let end = consume_trailing_newline(line_col_to_byte(source, line_starts, end_lc), source);
    (start, end)
}

fn impl_item_fn_range(
    f: &syn::ImplItemFn,
    source: &str,
    line_starts: &[usize],
) -> (usize, usize) {
    // The `fn` token doesn't cover the leading `pub` (or `pub(crate)`),
    // so resolve the main-start through the visibility token when one
    // is present. Without this the splice would leave a stray `pub `
    // before the deleted method.
    let main_start = vis_or_fallback(&f.vis, f.sig.fn_token.span.start());
    let start = leading_start(&f.attrs, main_start, source, line_starts);
    let end = consume_trailing_newline(line_col_to_byte(source, line_starts, f.span().end()), source);
    (start, end)
}

fn impl_item_const_range(
    c: &syn::ImplItemConst,
    source: &str,
    line_starts: &[usize],
) -> (usize, usize) {
    let main_start = vis_or_fallback(&c.vis, c.const_token.span.start());
    let start = leading_start(&c.attrs, main_start, source, line_starts);
    let end = consume_trailing_newline(line_col_to_byte(source, line_starts, c.span().end()), source);
    (start, end)
}

/// Returns the visibility's `pub` token start when present (so the
/// splice covers `pub fn …` instead of just `fn …`). Falls back to
/// `inherited` for crate-private items where there is no `pub` to
/// strip.
fn vis_or_fallback(vis: &Visibility, inherited: LineColumn) -> LineColumn {
    match vis {
        Visibility::Public(token) => token.span.start(),
        Visibility::Restricted(r) => r.pub_token.span.start(),
        Visibility::Inherited => inherited,
    }
}

/// Earliest byte that "belongs to" the declaration: the smallest of
/// (declaration's main token span start, first attribute span start).
fn leading_start(
    attrs: &[Attribute],
    main_start: LineColumn,
    source: &str,
    line_starts: &[usize],
) -> usize {
    let mut best = (main_start.line, main_start.column);
    for a in attrs {
        let s = a.span().start();
        if (s.line, s.column) < best {
            best = (s.line, s.column);
        }
    }
    line_col_to_byte(
        source,
        line_starts,
        LineColumn {
            line: best.0,
            column: best.1,
        },
    )
}

fn consume_trailing_newline(end: usize, source: &str) -> usize {
    if source.as_bytes().get(end) == Some(&b'\n') {
        end + 1
    } else {
        end
    }
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Fn(i) => &i.attrs,
        Item::Struct(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::Trait(i) => &i.attrs,
        Item::Type(i) => &i.attrs,
        Item::Const(i) => &i.attrs,
        Item::Static(i) => &i.attrs,
        Item::Union(i) => &i.attrs,
        _ => &[],
    }
}

fn type_path_last_segment(ty: &Type) -> Option<String> {
    if let Type::Path(tp) = ty {
        tp.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

/// Pre-computes byte offsets for the start of each line in `source`.
fn compute_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Converts a `LineColumn` (1-based line, 0-based char column) into
/// a UTF-8 byte offset. Walks the line's chars to handle non-ASCII
/// where char count ≠ byte count.
fn line_col_to_byte(source: &str, line_starts: &[usize], lc: LineColumn) -> usize {
    if lc.line == 0 || lc.line > line_starts.len() {
        return source.len();
    }
    let line_start = line_starts[lc.line - 1];
    let line_end = line_starts.get(lc.line).copied().unwrap_or(source.len());
    let line_text = &source[line_start..line_end];
    let mut chars_so_far = 0;
    for (idx, ch) in line_text.char_indices() {
        if chars_so_far == lc.column {
            return line_start + idx;
        }
        chars_so_far += 1;
        if ch == '\n' {
            break;
        }
    }
    line_start + line_text.len()
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
    fn line_col_to_byte_handles_ascii_and_multibyte() {
        let src = "fn f() {}\n// 日本語コメント\nfn g() {}\n";
        let starts = compute_line_starts(src);
        assert_eq!(
            line_col_to_byte(src, &starts, LineColumn { line: 1, column: 0 }),
            0
        );
        let g_offset = src.find("fn g() {}").unwrap();
        assert_eq!(
            line_col_to_byte(src, &starts, LineColumn { line: 3, column: 3 }),
            g_offset + 3
        );
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
        syn::parse_file(&after).expect("post-splice still parses");
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

    #[test]
    fn item_attrs_returns_empty_for_unsupported_variants() {
        let item: Item = syn::parse_quote!(
            extern "C" {
                fn extern_fn();
            }
        );
        assert!(item_attrs(&item).is_empty());
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
        let abs = write(&dir, "src/lib.rs", "pub enum E {\n    A,\n    B,\n    C,\n}\n");
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
        syn::parse_file(&after).expect("post-splice still parses");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_deletes_first_enum_variant() {
        let dir = tempdir("delete-variant-first");
        let abs = write(&dir, "src/lib.rs", "pub enum E {\n    A,\n    B,\n    C,\n}\n");
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
        syn::parse_file(&after).expect("still parses");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_deletes_last_enum_variant() {
        let dir = tempdir("delete-variant-last");
        // No trailing comma after the last variant on purpose — this
        // is the syntactic case that triggers the `prev.end → self.end`
        // branch.
        let abs = write(&dir, "src/lib.rs", "pub enum E {\n    A,\n    B,\n    C\n}\n");
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
        syn::parse_file(&after).expect("still parses");
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
        syn::parse_file(&after).expect("post-splice still parses");
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
        syn::parse_file(&after).expect("post-splice still parses");
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
