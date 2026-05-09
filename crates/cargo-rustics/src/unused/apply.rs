//! `cargo rustics unused --apply` — top-level orphan deletion.
//!
//! For every [`UnusedItem`] whose kind is a top-level declaration
//! (`fn` / `struct` / `enum` / `trait` / `type` / `const` / `static`
//! / `union`), look the item up in its source file's AST, compute the
//! byte range that covers the item *plus its leading attributes*, and
//! splice the range out. Methods, enum variants, and associated
//! consts are intentionally left in place — accurate splicing inside
//! an `impl` / `enum` block needs more delicate surgery (preserving
//! delimiters, comma handling, sibling-attr scope) and we'd rather
//! report them than risk a corrupted file.
//!
//! Safety rails before any edit lands:
//!
//! * `git_tree_is_clean` returns `false` if `git status --porcelain`
//!   prints anything; the CLI refuses to apply unless `--force` is
//!   set.
//! * Files under `tests/` or `**/integration_test/**` are skipped
//!   unless `--include-tests` is set; orphan helpers there are
//!   usually intentional.
//! * Splices are sorted descending by start byte, so earlier byte
//!   offsets remain valid as later ones are removed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use proc_macro2::LineColumn;
use syn::spanned::Spanned;
use syn::{Attribute, Item};

use super::UnusedItem;

/// Tally of what `apply` did during one invocation. Surfaces back to
/// the CLI so the operator can see how many edits landed and how many
/// reports were intentionally skipped.
#[derive(Debug, Default, Clone, Copy)]
pub struct Outcome {
    /// Top-level items deleted in place.
    pub deleted: usize,
    /// Number of distinct files mutated.
    pub touched_files: usize,
    /// Items that lived under `tests/` or `**/integration_test/**`
    /// and were skipped because `--include-tests` was not set.
    pub skipped_test_files: usize,
    /// Methods / enum variants / associated consts that were
    /// reported but whose deletion isn't yet supported.
    pub skipped_non_top_level: usize,
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
        // Not a git repo, or some other git error. Don't gate the
        // apply on that — the user is opting into the action.
        return Ok(true);
    }
    Ok(output.stdout.is_empty())
}

/// Top-level entry point. Filters `items` by file path, groups by
/// owning file, computes a deletion range for each top-level
/// candidate, and writes the splices back.
pub fn apply(workspace_root: &Path, items: &[UnusedItem], include_tests: bool) -> Result<Outcome> {
    let mut outcome = Outcome::default();
    let mut by_file: HashMap<PathBuf, Vec<&UnusedItem>> = HashMap::new();

    for item in items {
        if !is_top_level_kind(item.kind) {
            outcome.skipped_non_top_level += 1;
            continue;
        }
        if !include_tests && is_test_path(&item.file) {
            outcome.skipped_test_files += 1;
            continue;
        }
        let abs = workspace_root.join(&item.file);
        by_file.entry(abs).or_default().push(item);
    }

    for (abs, file_items) in by_file {
        let mutated = apply_file(&abs, &file_items)?;
        if mutated > 0 {
            outcome.deleted += mutated;
            outcome.touched_files += 1;
        }
    }
    Ok(outcome)
}

/// Returns `true` when `kind` (as set by [`super::collect_decls`]) is
/// a top-level declaration we can safely splice out at the file root.
/// Methods, enum variants, and associated consts have to live inside
/// a wrapping `impl` / `enum` — their byte spans don't survive raw
/// removal.
pub(super) fn is_top_level_kind(kind: &str) -> bool {
    matches!(
        kind,
        "fn" | "struct" | "enum" | "trait" | "type" | "const" | "static" | "union"
    )
}

/// Path filter mirroring dartrics's behaviour: every path under
/// `tests/` (workspace-level integration tests + per-crate
/// integration tests live there) and any path with a path component
/// `integration_test` is treated as test scaffolding.
pub(super) fn is_test_path(rel: &str) -> bool {
    rel.starts_with("tests/")
        || rel.contains("/tests/")
        || rel.contains("/integration_test/")
        || rel.starts_with("integration_test/")
}

/// Reads `path`, parses it, finds each requested item by `(name,
/// kind)`, and rewrites the file with those byte ranges spliced out.
/// Returns the number of items actually deleted (an item whose name
/// didn't resolve in the AST is skipped, not erroring out — the
/// detector and the apply pass walk independently and we want one
/// stale entry not to abort the rest of the run).
fn apply_file(path: &Path, items: &[&UnusedItem]) -> Result<usize> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("read {} for --apply", path.display()))?;
    let ast = syn::parse_file(&source)
        .with_context(|| format!("parse {} for --apply", path.display()))?;
    let line_starts = compute_line_starts(&source);

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for item in items {
        if let Some(range) = find_item_range(&ast, &source, &line_starts, &item.name, item.kind) {
            ranges.push(range);
        }
    }
    if ranges.is_empty() {
        return Ok(0);
    }
    // Sort descending by start byte so each splice doesn't shift the
    // indices of later splices.
    ranges.sort_by(|a, b| b.0.cmp(&a.0));
    let mut new_source = source;
    for (start, end) in &ranges {
        new_source.replace_range(start..end, "");
    }
    std::fs::write(path, &new_source)
        .with_context(|| format!("write {} after --apply", path.display()))?;
    Ok(ranges.len())
}

/// Searches the parsed file for a top-level [`Item`] whose ident
/// matches `name` and whose kind matches `kind`, then returns its
/// byte range in `source`. Recurses into inline `mod m { ... }` so a
/// nested decl is still locatable. Returns `None` if the item isn't
/// found — the detector might have surfaced a record from an earlier
/// state that the source has since drifted away from.
fn find_item_range(
    file: &syn::File,
    source: &str,
    line_starts: &[usize],
    name: &str,
    kind: &str,
) -> Option<(usize, usize)> {
    find_in_items(&file.items, source, line_starts, name, kind)
}

fn find_in_items(
    items: &[Item],
    source: &str,
    line_starts: &[usize],
    name: &str,
    kind: &str,
) -> Option<(usize, usize)> {
    for item in items {
        if matches_item(item, name, kind) {
            return Some(item_byte_range(item, source, line_starts));
        }
        if let Item::Mod(m) = item {
            if let Some((_, inner)) = &m.content {
                if let Some(r) = find_in_items(inner, source, line_starts, name, kind) {
                    return Some(r);
                }
            }
        }
    }
    None
}

fn matches_item(item: &Item, name: &str, kind: &str) -> bool {
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

/// Computes the byte range that covers `item` *and its leading
/// attributes* in `source`. The end is extended through the trailing
/// newline so a clean splice doesn't leave an orphaned blank line.
fn item_byte_range(item: &Item, source: &str, line_starts: &[usize]) -> (usize, usize) {
    let attrs = item_attrs(item);
    let main_span_start = item.span().start();
    let start_lc = attrs
        .first()
        .map(|a| a.span().start())
        .unwrap_or(main_span_start);
    let end_lc = item.span().end();

    let start = line_col_to_byte(source, line_starts, start_lc);
    let mut end = line_col_to_byte(source, line_starts, end_lc);
    // Consume the trailing newline if the splice would otherwise
    // leave one behind. We deliberately don't consume more than one —
    // that would collapse blank-line separators between unrelated
    // items.
    if source.as_bytes().get(end) == Some(&b'\n') {
        end += 1;
    }
    (start, end)
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

/// Pre-computes byte offsets for the start of each line in `source`.
/// Used to convert `proc_macro2::LineColumn` (1-based line, 0-based
/// char-column) into a UTF-8 byte offset.
fn compute_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0_usize];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Converts a `LineColumn` to a byte offset in `source`. Lines are
/// 1-based; columns are 0-based char offsets within the line. This
/// walks the line's chars to find the byte offset matching the char
/// column — needed for non-ASCII lines where char count ≠ byte count.
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
        // Don't walk past a newline — column should never reach there.
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
        UnusedItem {
            file: file.to_string(),
            line: 1,
            name: name.to_string(),
            kind,
            parent: None,
        }
    }

    #[test]
    fn is_top_level_kind_recognises_each_kind() {
        for k in [
            "fn", "struct", "enum", "trait", "type", "const", "static", "union",
        ] {
            assert!(is_top_level_kind(k), "{k} should be top-level");
        }
        for k in ["method", "variant", "assoc-const", "field"] {
            assert!(!is_top_level_kind(k), "{k} should not be top-level");
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
        // line 1, col 0 → byte 0
        assert_eq!(
            line_col_to_byte(src, &starts, LineColumn { line: 1, column: 0 }),
            0
        );
        // line 3, col 3 → start of 'g'. Line 3 starts after the
        // preceding multi-byte line, so the conversion must not
        // assume 1 byte per char.
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
        let outcome = apply(
            &dir,
            &[item("src/lib.rs", "drop_me", "fn")],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.touched_files, 1);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains("drop_me"), "after = {after:?}");
        assert!(after.contains("keep"));
        assert!(after.contains("other"));
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
        let abs = write(
            &dir,
            "tests/helpers.rs",
            "pub fn helper() {}\n",
        );
        let outcome = apply(
            &dir,
            &[item("tests/helpers.rs", "helper", "fn")],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 0);
        assert_eq!(outcome.skipped_test_files, 1);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(after.contains("helper"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_can_include_test_paths() {
        let dir = tempdir("incl-tests");
        let abs = write(
            &dir,
            "tests/helpers.rs",
            "pub fn helper() {}\n",
        );
        let outcome = apply(
            &dir,
            &[item("tests/helpers.rs", "helper", "fn")],
            true,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.skipped_test_files, 0);
        let after = std::fs::read_to_string(&abs).unwrap();
        assert!(!after.contains("helper"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_skips_methods_and_variants() {
        let dir = tempdir("skip-non-top");
        let _ = write(
            &dir,
            "src/lib.rs",
            "pub struct S; impl S { pub fn m() {} } pub enum E { A }\n",
        );
        let outcome = apply(
            &dir,
            &[
                item("src/lib.rs", "m", "method"),
                item("src/lib.rs", "A", "variant"),
            ],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 0);
        assert_eq!(outcome.skipped_non_top_level, 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_descending_order_keeps_offsets_valid() {
        // Two deletions in the same file: if we don't sort by
        // descending start, the second splice applies to a shifted
        // string and corrupts the output.
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
    fn apply_unknown_name_is_skipped_silently() {
        // The detector might surface a record from an earlier state.
        // Unknown names produce a "no match" return, not an error.
        let dir = tempdir("ghost-name");
        let _abs = write(&dir, "src/lib.rs", "pub fn alive() {}\n");
        let outcome = apply(
            &dir,
            &[item("src/lib.rs", "ghost", "fn")],
            false,
        )
        .unwrap();
        assert_eq!(outcome.deleted, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn git_tree_is_clean_returns_true_outside_git_repo() {
        let dir = tempdir("not-git");
        // No .git directory; `git status` exits non-zero, which we
        // treat as "not gating the apply".
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
}
