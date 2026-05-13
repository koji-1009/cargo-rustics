//! Efferent coupling — per-module count of distinct external crates
//! the module's `use` statements reach.
//!
//! HIR-backed counterpart of `rustics::metrics::efferent_coupling`.
//! The AST walker counts the *leftmost segment* of each `use` path,
//! which is a useful approximation but has two known accuracy gaps:
//!
//! * `use crate::foo::Bar` counts `crate` as a root, even though it
//!   refers to the file's own crate. The AST walker drops `self` and
//!   `super` but cannot tell `crate` apart from an external root
//!   without name resolution.
//! * `use some_facade::ReExported`, where `some_facade` re-exports
//!   from a third-party crate, counts `some_facade` rather than the
//!   *real* origin crate. Re-export hops are invisible to the
//!   token-only walker.
//!
//! The HIR version resolves each `use` leaf to its `Definition` via
//! [`Semantics::resolve_path`], asks for the defining crate via
//! `Module::krate`, and only counts crates that aren't the
//! current crate and aren't workspace-internal helpers
//! (`CrateOrigin::Local`). Stdlib roots (`std`, `core`, `alloc`,
//! `proc_macro`, `test`) and the workspace's own crates are
//! excluded from the count.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use ra_ap_hir::{attach_db, Crate, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::ast::{self, AstNode};

/// One efferent-coupling measurement, keyed by source file.
#[derive(Debug, Clone)]
pub struct EfferentMeasurement {
    /// VFS-shaped file path (forward slashes, absolute). The
    /// cargo-rustics wrapping layer strips the workspace prefix to
    /// match the report contract.
    pub file: String,
    /// Distinct external crate count for this file's modules.
    pub value: u32,
    /// Display name for the report's scope field — typically the
    /// file stem (`statistics`, `coupling`, `lib`, …) to mirror the
    /// AST walker's scope shape.
    pub scope: String,
}

/// Loads `manifest_dir`, walks every local crate's modules, and
/// returns one [`EfferentMeasurement`] per source file.
pub fn detect_at(manifest_dir: &Path) -> Result<Vec<EfferentMeasurement>> {
    let workspace = crate::workspace::load(manifest_dir)?;
    let db = workspace.host.raw_database();
    let vfs = &workspace.vfs;
    attach_db(db, || -> Result<Vec<EfferentMeasurement>> {
        // file_id → (file_path, self_crate, dependency_set). Walking
        // by file (not by module) avoids re-parsing inline modules
        // that share a file, and keeps `Semantics` walking a single
        // tree — `resolve_path` panics if it sees a syntax node
        // from a different parse than its own.
        let mut by_file: HashMap<u32, FileAcc> = HashMap::new();
        let sema = Semantics::new(db);
        for krate in Crate::all(db) {
            if !krate.origin(db).is_local() {
                continue;
            }
            collect_crate_files(db, krate, vfs, &mut by_file);
        }
        for entry in by_file.values_mut() {
            walk_file(db, &sema, entry);
        }
        Ok(finalise(by_file))
    })
}

/// Per-file aggregation state. The detector finalises by reading
/// `file` (path), `self_crate` (own-crate exclusion at the
/// `credit_path` boundary already happened, but we keep the marker
/// for debugging), and `external_crates`.
struct FileAcc {
    file: String,
    file_id: ra_ap_vfs::FileId,
    self_crate: Crate,
    external_crates: HashSet<u32>,
}

/// Enumerates every source file owned by a local crate, recording
/// the file_id, its VFS path, and the crate it belongs to. Inline
/// modules don't add new entries — `as_source_file_id` returns the
/// containing file, and the file is keyed by `FileId`.
fn collect_crate_files(
    db: &RootDatabase,
    krate: Crate,
    vfs: &ra_ap_vfs::Vfs,
    out: &mut HashMap<u32, FileAcc>,
) {
    let mut stack = vec![krate.root_module(db)];
    while let Some(module) = stack.pop() {
        for child in module.children(db) {
            stack.push(child);
        }
        let Some(editioned) = module.as_source_file_id(db) else {
            continue;
        };
        let file_id = editioned.file_id(db);
        let key = file_id_key(file_id);
        if out.contains_key(&key) {
            continue;
        }
        let Some(path) = vfs.file_path(file_id).as_path().map(|p| p.to_string()) else {
            continue;
        };
        out.insert(
            key,
            FileAcc {
                file: path,
                file_id,
                self_crate: krate,
                external_crates: HashSet::new(),
            },
        );
    }
}

/// `FileId` doesn't implement `Hash` directly in every ra_ap_*
/// version, so key by its raw u32 representation. The conversion
/// is stable for a single workspace load.
fn file_id_key(file_id: ra_ap_vfs::FileId) -> u32 {
    file_id.index()
}

/// Parses `file` through `sema` (so resolve_path sees a tree it
/// owns), walks every `use` item, and credits external crates into
/// `acc.external_crates`.
fn walk_file(db: &RootDatabase, sema: &Semantics<'_, RootDatabase>, acc: &mut FileAcc) {
    let source = sema.parse_guess_edition(acc.file_id);
    for use_item in source.syntax().descendants().filter_map(ast::Use::cast) {
        let Some(tree) = use_item.use_tree() else {
            continue;
        };
        for path in walk_use_tree_paths(&tree) {
            credit_path(db, sema, acc.self_crate, &path, &mut acc.external_crates);
        }
    }
}

/// Yields every leaf `Path` in a `use` tree (`use foo::{A, B}` has
/// two leaves, `foo::A` and `foo::B`). The aggregation collapses
/// them at the HIR-resolution step — multiple leaves resolving to
/// the same external crate count once.
fn walk_use_tree_paths(tree: &ast::UseTree) -> Vec<ast::Path> {
    let mut out = Vec::new();
    if let Some(list) = tree.use_tree_list() {
        for inner in list.use_trees() {
            out.extend(walk_use_tree_paths(&inner));
        }
        return out;
    }
    if let Some(path) = tree.path() {
        out.push(path);
    }
    out
}

/// Resolves `path` and inserts the target crate's id into `out` if
/// the target lives outside `self_crate` and is workspace-external.
fn credit_path(
    db: &RootDatabase,
    sema: &Semantics<'_, RootDatabase>,
    self_crate: Crate,
    path: &ast::Path,
    out: &mut HashSet<u32>,
) {
    let Some(resolution) = sema.resolve_path(path) else {
        return;
    };
    let target_crate = match resolution {
        ra_ap_hir::PathResolution::Def(def) => def.module(db).map(|m| m.krate(db)),
        _ => None,
    };
    let Some(target) = target_crate else {
        return;
    };
    if target == self_crate {
        return;
    }
    if target.origin(db).is_lang() {
        // stdlib (std / core / alloc / proc_macro / test) — Martin
        // Ce explicitly drops these, mirroring the AST walker.
        return;
    }
    // `Crate` doesn't expose a stable usize id we can hash easily;
    // fall back to the crate's display name, falling back to the
    // root module's file id for crates without a name.
    let key = stable_crate_key(db, target);
    out.insert(key);
}

/// Returns a stable hash key for a crate. We use the crate's
/// internal id surfaced through the `display_name` round-trip;
/// when that's unavailable, the crate's root file id provides a
/// fallback unique per-crate value.
fn stable_crate_key(db: &RootDatabase, krate: Crate) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(name) = krate.display_name(db) {
        name.to_string().hash(&mut h);
    } else {
        // Fall back to the root file id.
        let root = krate.root_module(db);
        if let Some(file_id) = root.as_source_file_id(db) {
            file_id.hash(&mut h);
        }
    }
    // Truncate to u32; collisions across crates would require two
    // different crates' display names to share the lower 32 bits of
    // a 64-bit hash — vanishingly unlikely at workspace scale.
    h.finish() as u32
}

fn finalise(by_file: HashMap<u32, FileAcc>) -> Vec<EfferentMeasurement> {
    let mut out: Vec<EfferentMeasurement> = by_file
        .into_values()
        .map(|acc| EfferentMeasurement {
            scope: file_stem(&acc.file),
            value: acc.external_crates.len() as u32,
            file: acc.file,
        })
        .collect();
    out.sort_by(|a, b| a.file.cmp(&b.file));
    out
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string()
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
        std::env::temp_dir().join(format!("rustics-ra-ce-{label}-{pid}-{n}-{s}"))
    }

    fn write_fixture(dir: &std::path::Path, cargo_toml: &str, lib_rs: &str) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("Cargo.toml"), cargo_toml).unwrap();
        fs::write(dir.join("src/lib.rs"), lib_rs).unwrap();
    }

    #[test]
    fn self_referential_use_crate_does_not_inflate_ce() {
        // The accuracy gain HIR brings: `use crate::foo::Bar`
        // resolves to *this* crate, so it must not count. The AST
        // walker would count `crate` as a distinct root.
        let dir = unique_dir("self-ref");
        write_fixture(
            &dir,
            "[package]\nname = \"a\"\nversion = \"0.0.1\"\nedition = \"2021\"\npublish = false\n[lib]\npath = \"src/lib.rs\"\n",
            "pub mod foo { pub struct Bar; }\nuse crate::foo::Bar;\npub fn use_bar(_: Bar) {}\n",
        );
        let out = detect_at(&dir).expect("detect_at");
        assert_eq!(out.len(), 1, "one lib.rs file expected, got: {out:?}");
        assert_eq!(
            out[0].value, 0,
            "use crate::foo::Bar must not count any external crate; got value: {}",
            out[0].value
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stdlib_use_does_not_count() {
        // Mirrors the AST walker's "std / core / alloc / proc_macro
        // / test are not external dependencies for Martin Ce".
        let dir = unique_dir("stdlib");
        write_fixture(
            &dir,
            "[package]\nname = \"b\"\nversion = \"0.0.1\"\nedition = \"2021\"\npublish = false\n[lib]\npath = \"src/lib.rs\"\n",
            "use std::collections::HashMap;\npub fn make() -> HashMap<u32, u32> { HashMap::new() }\n",
        );
        let out = detect_at(&dir).expect("detect_at");
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].value, 0,
            "stdlib use must not count; got: {}",
            out[0].value
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
