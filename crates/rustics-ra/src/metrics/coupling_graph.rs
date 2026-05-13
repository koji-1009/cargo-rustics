//! HIR-backed cross-file coupling: per-file Afferent Coupling (Ca)
//! and Instability (I = Ce_internal / (Ce_internal + Ca)).
//!
//! Replaces the AST-based `cargo_rustics::cross_file::coupling`
//! pass. The AST version walks each file's `use` items and resolves
//! the target module via longest-prefix matching against cargo
//! metadata's crate list, which has two failure modes the spike
//! triage flagged:
//!
//! * `use crate::foo::Bar` resolves correctly under prefix matching
//!   (the AST walker rewrites `crate::` to the source's own crate)
//!   *only when* the source's module path is known exactly, and the
//!   prefix-match returns the longest matching module file, not
//!   necessarily the file where `Bar` actually lives.
//! * `use re_exporting_crate::ReExported` where `ReExported` was
//!   re-exported from a different workspace crate: prefix matching
//!   counts the edge to `re_exporting_crate`'s lib.rs, not to the
//!   file where `ReExported` is *defined*. The Ca on the actual
//!   defining file is under-counted.
//!
//! HIR's `Semantics::resolve_path` returns the canonical
//! `Definition`; `Module::krate(db)` + `Module::as_source_file_id`
//! locate the file the resolved item lives in. Per-file out-edges
//! become the deduplicated set of those target files restricted to
//! workspace-local crates. Ca is the reverse adjacency.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use ra_ap_hir::{attach_db, Crate, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::ast::{self, AstNode};

/// One per-file coupling measurement.
#[derive(Debug, Clone)]
pub struct CouplingMeasurement {
    /// VFS-shaped file path (forward slashes, absolute).
    pub file: String,
    /// Display name for the report's scope field — file stem.
    pub scope: String,
    /// Afferent count (Ca): how many *other* workspace files depend
    /// on this one through a `use` statement that resolves into it.
    pub afferent: u32,
    /// Workspace-internal efferent count: how many distinct other
    /// workspace files this file's `use` statements reach. Used as
    /// the numerator of `instability` — separate from the Phase 2
    /// efferent-coupling metric, which counts *external* crates.
    pub efferent_internal: u32,
    /// Martin Instability: `Ce_internal / (Ce_internal + Ca)`.
    /// Files with zero in-edges and zero out-edges report `0.0` —
    /// mirroring the existing AST lens's convention.
    pub instability: f64,
}

/// Loads `manifest_dir` and emits one [`CouplingMeasurement`] per
/// workspace-local source file.
pub fn detect_at(manifest_dir: &Path) -> Result<Vec<CouplingMeasurement>> {
    let workspace = crate::workspace::load(manifest_dir)?;
    let db = workspace.host.raw_database();
    let vfs = &workspace.vfs;
    attach_db(db, || -> Result<Vec<CouplingMeasurement>> {
        // file_id_key → (file_path, owning_crate). Every workspace-
        // local file is enumerated up front so the dependency graph
        // can be built over a stable set of vertices.
        let files = enumerate_workspace_files(db, vfs);
        let sema = Semantics::new(db);
        let edges = build_dependency_graph(db, &sema, &files);
        Ok(finalise(&files, &edges))
    })
}

#[derive(Clone)]
struct FileEntry {
    path: String,
    file_id: ra_ap_vfs::FileId,
}

fn enumerate_workspace_files(
    db: &RootDatabase,
    vfs: &ra_ap_vfs::Vfs,
) -> HashMap<u32, FileEntry> {
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

/// For each source file, the set of *other* workspace files it
/// depends on through a HIR-resolved `use` statement. Edges crossing
/// crate boundaries within the workspace are included; edges to
/// stdlib / non-local crates are not. Self-edges (a `use` that
/// resolves into the same file) are excluded so the per-file graph
/// has no loops.
fn build_dependency_graph(
    db: &RootDatabase,
    sema: &Semantics<'_, RootDatabase>,
    files: &HashMap<u32, FileEntry>,
) -> HashMap<u32, HashSet<u32>> {
    let mut edges: HashMap<u32, HashSet<u32>> = HashMap::with_capacity(files.len());
    for (&src_key, entry) in files {
        let parsed = sema.parse_guess_edition(entry.file_id);
        let mut targets: HashSet<u32> = HashSet::new();
        for use_item in parsed.syntax().descendants().filter_map(ast::Use::cast) {
            let Some(tree) = use_item.use_tree() else {
                continue;
            };
            for path in walk_use_tree_paths(&tree) {
                resolve_target(db, sema, &path, files, src_key, &mut targets);
            }
        }
        edges.insert(src_key, targets);
    }
    edges
}

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

/// Resolves `path` via HIR and, if the target lives in another
/// workspace file we've enumerated, inserts that file's key into
/// `targets`. Stdlib targets, external crate targets, and self-
/// targets (`use crate::self_module::*`) are skipped.
fn resolve_target(
    db: &RootDatabase,
    sema: &Semantics<'_, RootDatabase>,
    path: &ast::Path,
    files: &HashMap<u32, FileEntry>,
    src_key: u32,
    targets: &mut HashSet<u32>,
) {
    let Some(resolution) = sema.resolve_path(path) else {
        return;
    };
    let target_module = match resolution {
        ra_ap_hir::PathResolution::Def(def) => def.module(db),
        _ => return,
    };
    let Some(target_module) = target_module else {
        return;
    };
    let target_crate = target_module.krate(db);
    if !target_crate.origin(db).is_local() {
        return;
    }
    let Some(editioned) = target_module.as_source_file_id(db) else {
        return;
    };
    let target_key = editioned.file_id(db).index();
    if target_key == src_key {
        return;
    }
    if files.contains_key(&target_key) {
        targets.insert(target_key);
    }
}

fn finalise(
    files: &HashMap<u32, FileEntry>,
    edges: &HashMap<u32, HashSet<u32>>,
) -> Vec<CouplingMeasurement> {
    // Reverse the adjacency for Ca.
    let mut afferent: HashMap<u32, u32> = HashMap::new();
    for targets in edges.values() {
        for target in targets {
            *afferent.entry(*target).or_default() += 1;
        }
    }
    let mut out: Vec<CouplingMeasurement> = files
        .iter()
        .map(|(key, entry)| {
            let ca = afferent.get(key).copied().unwrap_or(0);
            let ce_internal = edges.get(key).map(|s| s.len()).unwrap_or(0) as u32;
            let total = ce_internal + ca;
            let instability = if total == 0 {
                0.0
            } else {
                f64::from(ce_internal) / f64::from(total)
            };
            CouplingMeasurement {
                file: entry.path.clone(),
                scope: file_stem(&entry.path),
                afferent: ca,
                efferent_internal: ce_internal,
                instability,
            }
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
        std::env::temp_dir().join(format!("rustics-ra-coupling-{label}-{pid}-{n}-{s}"))
    }

    fn write_fixture(dir: &std::path::Path, cargo_toml: &str, files: &[(&str, &str)]) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("Cargo.toml"), cargo_toml).unwrap();
        for (rel, body) in files {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, body).unwrap();
        }
    }

    fn find<'a>(out: &'a [CouplingMeasurement], stem: &str) -> &'a CouplingMeasurement {
        out.iter()
            .find(|m| m.scope == stem)
            .unwrap_or_else(|| panic!("no measurement for {stem}: {out:?}"))
    }

    #[test]
    fn one_file_depends_on_another_yields_ca_one_and_correct_instability() {
        // `b.rs` defines `Bar`; `a.rs` uses it. Expectation:
        //   a.rs: Ce_internal=1, Ca=0, I = 1 / (1+0) = 1.0
        //   b.rs: Ce_internal=0, Ca=1, I = 0 / (0+1) = 0.0
        //   lib.rs: pure module declarations, no use → I=0
        let dir = unique_dir("two-files");
        write_fixture(
            &dir,
            "[package]\nname = \"k\"\nversion = \"0.0.1\"\nedition = \"2021\"\npublish = false\n[lib]\npath = \"src/lib.rs\"\n",
            &[
                ("src/lib.rs", "pub mod a;\npub mod b;\n"),
                ("src/a.rs", "use crate::b::Bar;\npub fn pull(_: Bar) {}\n"),
                ("src/b.rs", "pub struct Bar;\n"),
            ],
        );
        let out = detect_at(&dir).expect("detect_at");
        let a = find(&out, "a");
        let b = find(&out, "b");
        assert_eq!(a.afferent, 0, "a.rs has no incoming edges; got {a:?}");
        assert_eq!(a.efferent_internal, 1, "a.rs depends on b.rs; got {a:?}");
        assert!(
            (a.instability - 1.0).abs() < 1e-9,
            "a.rs is fully unstable: {}",
            a.instability
        );
        assert_eq!(b.afferent, 1, "b.rs is depended on by a.rs; got {b:?}");
        assert_eq!(b.efferent_internal, 0, "b.rs has no use stmts");
        assert!(
            b.instability.abs() < 1e-9,
            "b.rs is fully stable: {}",
            b.instability
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stdlib_use_does_not_create_workspace_edge() {
        // `use std::collections::HashMap` must not contribute to
        // any workspace edge; the lone file should report Ca=0 /
        // Ce_internal=0 / I=0 (the zero-edge convention).
        let dir = unique_dir("stdlib");
        write_fixture(
            &dir,
            "[package]\nname = \"k\"\nversion = \"0.0.1\"\nedition = \"2021\"\npublish = false\n[lib]\npath = \"src/lib.rs\"\n",
            &[(
                "src/lib.rs",
                "use std::collections::HashMap;\npub fn make() -> HashMap<u32, u32> { HashMap::new() }\n",
            )],
        );
        let out = detect_at(&dir).expect("detect_at");
        let lib = find(&out, "lib");
        assert_eq!(lib.afferent, 0);
        assert_eq!(lib.efferent_internal, 0);
        assert!(lib.instability.abs() < 1e-9);
        let _ = fs::remove_dir_all(&dir);
    }
}
