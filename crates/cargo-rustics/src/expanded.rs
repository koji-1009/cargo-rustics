//! `--expanded-macros` — re-run lenses on cargo-expand's macro-expanded
//! output.
//!
//! Enumerates workspace members via `cargo metadata`, spawns
//! `cargo expand --lib -p <pkg>` for every package with a lib target
//! and `cargo expand --bin <name> -p <pkg>` for every binary target,
//! and concatenates the captured stdouts into one synthetic file fed
//! back through the file walker. Lens output then reflects the
//! post-expansion AST — closing the Layer 1 blind spot that
//! `syn::Visit` doesn't enter macro bodies, so a `c.method()` call
//! inside `eprintln!(…)` is invisible until expansion happens.
//!
//! `cargo expand` itself does not accept a workspace root as a
//! virtual manifest — it requires a specific `-p <name>` package.
//! The earlier single `cargo expand --lib` at the workspace root
//! failed with "is a virtual manifest, but this command requires
//! running against an actual package" on any multi-crate workspace
//! (including this repository's self-application). Iterating
//! members is the documented intent the implementation now honours.
//!
//! The integration is opt-in. If `cargo-expand` is not installed we
//! print a stderr note and return `Ok(None)`; the analyzer falls
//! back to the un-expanded source.
//!
//! ## Testability
//!
//! The subprocess invocation is split into an [`ExpandRunner`] trait
//! so tests can drive every code path — availability check, success,
//! non-zero exit, non-UTF-8 output, write failure — without needing
//! a real `cargo-expand` install on the test host. Production uses
//! the [`Cargo`] runner which delegates to `std::process::Command`
//! and `cargo_metadata`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::discover::DiscoveredFile;

/// One `cargo expand` invocation target. Production code derives
/// these from `cargo metadata`'s workspace-member target list;
/// `Lib` covers crates with a `[lib]` section, `Bin(name)` covers
/// every `[[bin]]`. Other kinds (`example`, `test`, `bench`,
/// `proc-macro`) are not currently expanded — proc-macros run at
/// build time and tests/examples aren't part of the public surface
/// the unused detector reports against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetKind {
    /// `cargo expand --lib -p <package>`.
    Lib,
    /// `cargo expand --bin <name> -p <package>`.
    Bin(String),
}

/// One workspace target to expand: a `(package, kind)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandTarget {
    /// Cargo package name (matches `cargo metadata`'s `package.name`).
    pub package: String,
    /// Which `cargo expand` flag set to invoke.
    pub kind: TargetKind,
}

/// Runs `cargo expand` for every lib + bin target in every workspace
/// member, concatenates the captured stdouts, and returns the
/// aggregated source as a single synthetic file. Returns `Ok(None)`
/// if `cargo-expand` is not installed or if the workspace yields zero
/// expandable targets; returns `Err(_)` on subprocess-spawn failures
/// (which the caller surfaces as exit-70 internal errors).
pub fn expand_workspace(workspace_root: &Path) -> Result<Option<DiscoveredFile>> {
    expand_workspace_with(workspace_root, &Cargo)
}

/// Same as [`expand_workspace`] but with an injectable subprocess
/// runner — tests pass a fake; production (via [`expand_workspace`])
/// uses [`Cargo`].
pub fn expand_workspace_with(
    workspace_root: &Path,
    runner: &dyn ExpandRunner,
) -> Result<Option<DiscoveredFile>> {
    if !runner.cargo_expand_available() {
        warn_unavailable();
        return Ok(None);
    }
    let targets = match runner.enumerate_targets(workspace_root) {
        Ok(t) => t,
        Err(e) => {
            // `cargo metadata` failures (no Cargo.toml, broken
            // manifest, etc.) are recoverable for the `--expanded-
            // macros` leg — the un-expanded walker still runs. Log
            // and fall back rather than propagating an exit-70.
            eprintln!(
                "rustics: cargo expand could not enumerate workspace targets at {}: {e:#}",
                workspace_root.display()
            );
            return Ok(None);
        }
    };
    if targets.is_empty() {
        // No lib / bin targets — nothing to expand. The un-expanded
        // walker still runs, so this is not an error.
        return Ok(None);
    }
    let Some(source) = run_and_decode(runner, workspace_root, &targets)? else {
        return Ok(None);
    };
    let synthetic = synthetic_file_path(workspace_root);
    if !persist(&synthetic, &source) {
        return Ok(None);
    }
    Ok(Some(DiscoveredFile {
        absolute: synthetic,
        relative: ".rustics-expanded.rs".to_string(),
    }))
}

fn warn_unavailable() {
    eprintln!(
        "rustics: --expanded-macros set but `cargo expand` is not available. \
         Install with `cargo install cargo-expand` and re-run. Continuing on \
         the un-expanded AST."
    );
}

/// Iterates `targets`, invokes the runner once per target, and
/// concatenates the successful stdouts into one decoded string.
/// Returns `Ok(None)` on the recoverable failure modes (every target
/// failed, or the aggregated stdout was not UTF-8) so the caller can
/// fall back to the un-expanded AST. `Err` is reserved for
/// "subprocess could not be started", which is unrecoverable.
///
/// Partial failures (some targets expand, some don't) keep the
/// successful output and surface the failing targets' stderr — that
/// way a single misbehaving package doesn't take the whole report
/// down.
fn run_and_decode(
    runner: &dyn ExpandRunner,
    workspace_root: &Path,
    targets: &[ExpandTarget],
) -> Result<Option<String>> {
    let mut combined: Vec<u8> = Vec::new();
    let mut had_any_success = false;
    for target in targets {
        let output = runner
            .run_cargo_expand(workspace_root, target)
            .with_context(|| {
                format!(
                    "invoke cargo expand for {} ({:?}) at {}",
                    target.package,
                    target.kind,
                    workspace_root.display()
                )
            })?;
        if !output.success {
            eprintln!(
                "rustics: cargo expand failed for {} ({:?}): {}",
                target.package, target.kind, output.stderr
            );
            continue;
        }
        had_any_success = true;
        if !combined.is_empty() {
            combined.push(b'\n');
        }
        combined.extend(output.stdout);
    }
    if !had_any_success {
        return Ok(None);
    }
    match String::from_utf8(combined) {
        Ok(s) => Ok(Some(s)),
        Err(_) => {
            eprintln!("rustics: aggregated cargo expand output was not UTF-8");
            Ok(None)
        }
    }
}

/// Writes the expanded source to `path`, creating the parent directory
/// if missing. Returns `false` on any IO failure (best-effort —
/// failure is non-fatal; we just continue without the expanded file).
fn persist(path: &Path, source: &str) -> bool {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(path, source) {
        Ok(_) => true,
        Err(e) => {
            eprintln!(
                "rustics: cargo expand could not persist expanded source at {}: {e}",
                path.display()
            );
            false
        }
    }
}

/// Returns the temporary path that holds the persisted expanded
/// source. Stable so successive runs overwrite the same file rather
/// than littering `target/`.
pub fn synthetic_file_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join("target").join(".rustics-expanded.rs")
}

/// Captured output of one `cargo expand` invocation.
#[derive(Clone)]
pub struct ExpandOutput {
    /// Whether the subprocess exited with status 0.
    pub success: bool,
    /// Captured stdout — held as raw bytes because `cargo expand`'s
    /// output is *almost* always UTF-8 but the analyzer is robust
    /// against the off chance it isn't.
    pub stdout: Vec<u8>,
    /// Captured stderr (already lossy-decoded).
    pub stderr: String,
}

/// Subprocess runner abstraction. The production impl is [`Cargo`];
/// tests use a fake to simulate every branch of `expand_workspace_with`.
pub trait ExpandRunner {
    /// Returns `true` iff `cargo expand --help` succeeds.
    fn cargo_expand_available(&self) -> bool;
    /// Enumerates every lib + bin target in the workspace rooted at
    /// `workspace_root`. Production resolves this via
    /// `cargo metadata`; tests can return a hand-built list.
    fn enumerate_targets(&self, workspace_root: &Path) -> Result<Vec<ExpandTarget>>;
    /// Runs `cargo expand` for one target and captures the output.
    /// The error variant is reserved for the case where the
    /// subprocess could not be *started*; a non-zero exit is
    /// reported inside the [`ExpandOutput`].
    fn run_cargo_expand(
        &self,
        workspace_root: &Path,
        target: &ExpandTarget,
    ) -> std::io::Result<ExpandOutput>;
}

/// Production runner — delegates to `std::process::Command` and
/// `cargo_metadata`.
pub struct Cargo;

impl ExpandRunner for Cargo {
    fn cargo_expand_available(&self) -> bool {
        Command::new("cargo")
            .args(["expand", "--help"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn enumerate_targets(&self, workspace_root: &Path) -> Result<Vec<ExpandTarget>> {
        let metadata = cargo_metadata::MetadataCommand::new()
            .current_dir(workspace_root)
            .no_deps()
            .exec()
            .with_context(|| format!("cargo metadata at {}", workspace_root.display()))?;
        let member_ids: std::collections::HashSet<&cargo_metadata::PackageId> =
            metadata.workspace_members.iter().collect();
        let mut out = Vec::new();
        for pkg in &metadata.packages {
            if !member_ids.contains(&pkg.id) {
                continue;
            }
            for target in &pkg.targets {
                if let Some(kind) = classify_target_kind(target) {
                    out.push(ExpandTarget {
                        package: pkg.name.to_string(),
                        kind,
                    });
                }
            }
        }
        Ok(out)
    }

    fn run_cargo_expand(
        &self,
        workspace_root: &Path,
        target: &ExpandTarget,
    ) -> std::io::Result<ExpandOutput> {
        let mut args: Vec<&str> = vec!["expand"];
        let bin_name;
        match &target.kind {
            TargetKind::Lib => args.push("--lib"),
            TargetKind::Bin(name) => {
                args.push("--bin");
                bin_name = name.clone();
                args.push(&bin_name);
            }
        }
        args.push("-p");
        args.push(&target.package);
        let out = Command::new("cargo")
            .args(&args)
            .current_dir(workspace_root)
            .output()?;
        Ok(ExpandOutput {
            success: out.status.success(),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Maps a `cargo_metadata::Target` to the [`TargetKind`] we expand.
/// Returns `None` for kinds the unused detector doesn't care about
/// (`example`, `test`, `bench`, `proc-macro`, `custom-build`).
/// `lib` wins over `cdylib` / `staticlib` siblings — they share the
/// same source so we expand once.
fn classify_target_kind(target: &cargo_metadata::Target) -> Option<TargetKind> {
    use cargo_metadata::TargetKind as CmKind;
    if target.kind.iter().any(|k| matches!(k, CmKind::Lib)) {
        return Some(TargetKind::Lib);
    }
    if target.kind.iter().any(|k| matches!(k, CmKind::Bin)) {
        return Some(TargetKind::Bin(target.name.clone()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn synthetic_file_path_is_under_target() {
        let p = synthetic_file_path(Path::new("/ws"));
        assert!(p.ends_with("target/.rustics-expanded.rs"));
    }

    /// Fake runner — records each method's call count and returns
    /// values configured per-test. Default target list is a single
    /// lib so the existing tests' contract (one invocation per call)
    /// stays meaningful.
    struct FakeRunner {
        available: bool,
        targets: Vec<ExpandTarget>,
        out: ExpandOutput,
        run_calls: Cell<u32>,
        // None → return Err(io::Error) instead of Ok(out).
        spawn_error: bool,
    }

    impl FakeRunner {
        fn single_lib(available: bool, out: ExpandOutput, spawn_error: bool) -> Self {
            Self {
                available,
                targets: vec![ExpandTarget {
                    package: "fake".into(),
                    kind: TargetKind::Lib,
                }],
                out,
                run_calls: Cell::new(0),
                spawn_error,
            }
        }
    }

    impl ExpandRunner for FakeRunner {
        fn cargo_expand_available(&self) -> bool {
            self.available
        }
        fn enumerate_targets(&self, _: &Path) -> Result<Vec<ExpandTarget>> {
            Ok(self.targets.clone())
        }
        fn run_cargo_expand(&self, _: &Path, _: &ExpandTarget) -> std::io::Result<ExpandOutput> {
            self.run_calls.set(self.run_calls.get() + 1);
            if self.spawn_error {
                return Err(std::io::Error::other("simulated spawn failure"));
            }
            Ok(ExpandOutput {
                success: self.out.success,
                stdout: self.out.stdout.clone(),
                stderr: self.out.stderr.clone(),
            })
        }
    }

    fn tempdir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rustics-expanded-{label}-{pid}-{n}-{seq}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn returns_none_when_cargo_expand_unavailable() {
        let r = FakeRunner::single_lib(
            false,
            ExpandOutput {
                success: true,
                stdout: vec![],
                stderr: String::new(),
            },
            false,
        );
        let dir = tempdir("nounav");
        let result = expand_workspace_with(&dir, &r).unwrap();
        assert!(result.is_none());
        assert_eq!(r.run_calls.get(), 0, "must short-circuit before run");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn happy_path_writes_synthetic_file() {
        let r = FakeRunner::single_lib(
            true,
            ExpandOutput {
                success: true,
                stdout: b"fn expanded() {}\n".to_vec(),
                stderr: String::new(),
            },
            false,
        );
        let dir = tempdir("happy");
        let result = expand_workspace_with(&dir, &r).unwrap().expect("Some");
        assert_eq!(result.relative, ".rustics-expanded.rs");
        assert_eq!(result.absolute, synthetic_file_path(&dir));
        let written = std::fs::read_to_string(&result.absolute).unwrap();
        assert!(written.contains("fn expanded()"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_zero_exit_returns_none_with_stderr_logged() {
        let r = FakeRunner::single_lib(
            true,
            ExpandOutput {
                success: false,
                stdout: vec![],
                stderr: "broken manifest".into(),
            },
            false,
        );
        let dir = tempdir("badexit");
        assert!(expand_workspace_with(&dir, &r).unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_utf8_stdout_returns_none() {
        let r = FakeRunner::single_lib(
            true,
            ExpandOutput {
                success: true,
                stdout: vec![0xff, 0xfe, 0xfd],
                stderr: String::new(),
            },
            false,
        );
        let dir = tempdir("nonutf8");
        assert!(expand_workspace_with(&dir, &r).unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spawn_error_propagates_with_context() {
        let r = FakeRunner::single_lib(
            true,
            ExpandOutput {
                success: true,
                stdout: vec![],
                stderr: String::new(),
            },
            true,
        );
        let dir = tempdir("spawn");
        let err = expand_workspace_with(&dir, &r).unwrap_err();
        assert!(format!("{err:#}").contains("invoke cargo expand"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cargo_runner_availability_does_not_panic() {
        // Just exercise the production path: it should answer either
        // way without panicking on the host. We don't assert the value
        // because the test machine may or may not have cargo-expand.
        let _ = Cargo.cargo_expand_available();
    }

    #[test]
    fn persist_returns_false_when_write_fails() {
        // The expanded-macros pipeline is "best-effort": if we can't
        // write the synthetic file (parent path is occupied by a
        // regular file, disk is full, …) we surface a stderr note and
        // return `false`, and the caller falls back to the un-expanded
        // AST. Trigger the false branch by occupying the would-be
        // parent directory with a regular file so `create_dir_all` is
        // silently no-op'd and `write` fails because the parent isn't
        // a directory.
        let dir = tempdir("persist-fail");
        // Block the target/ slot with a regular file so the parent of
        // synthetic_file_path() can't be created.
        std::fs::write(dir.join("target"), "occupied").unwrap();
        let path = synthetic_file_path(&dir);
        let ok = persist(&path, "fn expanded() {}\n");
        assert!(
            !ok,
            "expected persist to return false when parent isn't a directory"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expand_workspace_returns_none_when_persist_fails() {
        // End-to-end variant: the runner reports success but `persist`
        // can't land the file. The public entry point must still
        // return `Ok(None)` (best-effort skip), not propagate the
        // write error to the caller — `--expanded-macros` is
        // informative, not load-bearing.
        let r = FakeRunner::single_lib(
            true,
            ExpandOutput {
                success: true,
                stdout: b"fn expanded() {}\n".to_vec(),
                stderr: String::new(),
            },
            false,
        );
        let dir = tempdir("e2e-persist-fail");
        std::fs::write(dir.join("target"), "occupied").unwrap();
        let result = expand_workspace_with(&dir, &r).unwrap();
        assert!(result.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expand_workspace_delegates_to_cargo() {
        // Smoke test of the public entry — runs against a tempdir
        // (which has no Cargo.toml). The Cargo runner then either
        // reports unavailable (Ok(None)) or runs cargo expand which
        // also returns Ok(None) because there's no manifest. Either
        // way the call returns Ok(None) without panicking.
        let dir = tempdir("delegate");
        let result = expand_workspace(&dir).unwrap();
        assert!(result.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Multi-target runner: each invocation returns a unique stdout
    /// keyed by the order the iteration happens to visit. Lets us
    /// assert "every target ran" + "outputs concatenate".
    struct MultiTargetRunner {
        targets: Vec<ExpandTarget>,
        outputs: std::cell::RefCell<std::collections::HashMap<String, ExpandOutput>>,
        run_calls: Cell<u32>,
    }

    impl ExpandRunner for MultiTargetRunner {
        fn cargo_expand_available(&self) -> bool {
            true
        }
        fn enumerate_targets(&self, _: &Path) -> Result<Vec<ExpandTarget>> {
            Ok(self.targets.clone())
        }
        fn run_cargo_expand(
            &self,
            _: &Path,
            target: &ExpandTarget,
        ) -> std::io::Result<ExpandOutput> {
            self.run_calls.set(self.run_calls.get() + 1);
            let key = format!("{}:{:?}", target.package, target.kind);
            Ok(self
                .outputs
                .borrow()
                .get(&key)
                .cloned()
                .unwrap_or(ExpandOutput {
                    success: false,
                    stdout: vec![],
                    stderr: format!("no fake configured for {key}"),
                }))
        }
    }

    #[test]
    fn enumerates_and_concatenates_per_target_output() {
        // The bug this whole rewrite addresses: workspaces with
        // multiple members must produce expanded output covering
        // every package, not just one. Each target's stdout must
        // appear in the aggregated source so lenses see post-
        // expansion code for the whole tree.
        let targets = vec![
            ExpandTarget {
                package: "lib_pkg".into(),
                kind: TargetKind::Lib,
            },
            ExpandTarget {
                package: "bin_pkg".into(),
                kind: TargetKind::Bin("bin_pkg".into()),
            },
        ];
        let outputs: std::collections::HashMap<String, ExpandOutput> = [
            (
                "lib_pkg:Lib".to_string(),
                ExpandOutput {
                    success: true,
                    stdout: b"// from lib_pkg\nfn lib_marker() {}\n".to_vec(),
                    stderr: String::new(),
                },
            ),
            (
                "bin_pkg:Bin(\"bin_pkg\")".to_string(),
                ExpandOutput {
                    success: true,
                    stdout: b"// from bin_pkg\nfn bin_marker() {}\n".to_vec(),
                    stderr: String::new(),
                },
            ),
        ]
        .into_iter()
        .collect();
        let runner = MultiTargetRunner {
            targets,
            outputs: std::cell::RefCell::new(outputs),
            run_calls: Cell::new(0),
        };
        let dir = tempdir("multi-target");
        let synth = expand_workspace_with(&dir, &runner)
            .unwrap()
            .expect("multi-target run should produce a synthetic file");
        let body = std::fs::read_to_string(&synth.absolute).unwrap();
        assert!(body.contains("fn lib_marker"), "lib output missing: {body}");
        assert!(body.contains("fn bin_marker"), "bin output missing: {body}");
        assert_eq!(runner.run_calls.get(), 2, "ran once per target");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn one_failing_target_does_not_lose_the_other() {
        // Partial-failure contract: if lib_pkg expands cleanly but
        // bin_pkg's `cargo expand` exits non-zero (e.g. user has a
        // bin target that fails to compile right now), we keep the
        // lib output rather than dropping the whole report.
        let targets = vec![
            ExpandTarget {
                package: "lib_pkg".into(),
                kind: TargetKind::Lib,
            },
            ExpandTarget {
                package: "bin_pkg".into(),
                kind: TargetKind::Bin("bin_pkg".into()),
            },
        ];
        let outputs: std::collections::HashMap<String, ExpandOutput> = [
            (
                "lib_pkg:Lib".to_string(),
                ExpandOutput {
                    success: true,
                    stdout: b"fn lib_marker() {}\n".to_vec(),
                    stderr: String::new(),
                },
            ),
            (
                "bin_pkg:Bin(\"bin_pkg\")".to_string(),
                ExpandOutput {
                    success: false,
                    stdout: vec![],
                    stderr: "compile error".into(),
                },
            ),
        ]
        .into_iter()
        .collect();
        let runner = MultiTargetRunner {
            targets,
            outputs: std::cell::RefCell::new(outputs),
            run_calls: Cell::new(0),
        };
        let dir = tempdir("partial-fail");
        let synth = expand_workspace_with(&dir, &runner)
            .unwrap()
            .expect("partial-success should still produce a file");
        let body = std::fs::read_to_string(&synth.absolute).unwrap();
        assert!(body.contains("fn lib_marker"));
        assert!(
            !body.contains("compile error"),
            "stderr text must not leak into stdout aggregate"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn all_failing_targets_returns_none() {
        // When every target fails, we have no expanded source to
        // offer. The caller must fall back to the un-expanded AST.
        let targets = vec![ExpandTarget {
            package: "lib_pkg".into(),
            kind: TargetKind::Lib,
        }];
        let outputs: std::collections::HashMap<String, ExpandOutput> = [(
            "lib_pkg:Lib".to_string(),
            ExpandOutput {
                success: false,
                stdout: vec![],
                stderr: "broken".into(),
            },
        )]
        .into_iter()
        .collect();
        let runner = MultiTargetRunner {
            targets,
            outputs: std::cell::RefCell::new(outputs),
            run_calls: Cell::new(0),
        };
        let dir = tempdir("all-fail");
        let result = expand_workspace_with(&dir, &runner).unwrap();
        assert!(result.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_target_list_returns_none() {
        // Edge case — workspace with members but none have a lib or
        // bin target (a workspace of proc-macro / test-only members
        // exists in the wild). Skip expansion gracefully.
        let runner = FakeRunner {
            available: true,
            targets: vec![],
            out: ExpandOutput {
                success: true,
                stdout: b"unused".to_vec(),
                stderr: String::new(),
            },
            run_calls: Cell::new(0),
            spawn_error: false,
        };
        let dir = tempdir("no-targets");
        let result = expand_workspace_with(&dir, &runner).unwrap();
        assert!(result.is_none());
        assert_eq!(runner.run_calls.get(), 0, "no targets → no invocations");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn classify_target_kind_recognises_lib_and_bin() {
        // We can't construct `cargo_metadata::Target` from outside
        // the crate, so this test relies on the serde round-trip
        // path the rest of the codebase uses to build fixture
        // metadata. Asserting the classifier on synthetic JSON
        // keeps the contract local — the dispatcher must say `Lib`
        // for any target whose `kind` array includes `"lib"`, and
        // `Bin(<name>)` for any target whose `kind` array includes
        // `"bin"`. Other kinds map to `None`.
        let lib: cargo_metadata::Target = serde_json::from_value(serde_json::json!({
            "name": "anything",
            "kind": ["lib"],
            "crate_types": ["lib"],
            "src_path": "/tmp/lib.rs",
            "edition": "2021",
            "doctest": false,
            "test": false,
            "doc": false,
        }))
        .unwrap();
        let bin: cargo_metadata::Target = serde_json::from_value(serde_json::json!({
            "name": "tool",
            "kind": ["bin"],
            "crate_types": ["bin"],
            "src_path": "/tmp/main.rs",
            "edition": "2021",
            "doctest": false,
            "test": false,
            "doc": false,
        }))
        .unwrap();
        let example: cargo_metadata::Target = serde_json::from_value(serde_json::json!({
            "name": "demo",
            "kind": ["example"],
            "crate_types": ["bin"],
            "src_path": "/tmp/demo.rs",
            "edition": "2021",
            "doctest": false,
            "test": false,
            "doc": false,
        }))
        .unwrap();
        assert_eq!(classify_target_kind(&lib), Some(TargetKind::Lib));
        assert_eq!(
            classify_target_kind(&bin),
            Some(TargetKind::Bin("tool".to_string()))
        );
        assert_eq!(classify_target_kind(&example), None);
    }
}
