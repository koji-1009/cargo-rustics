//! `--expanded-macros` — re-run lenses on cargo-expand's macro-expanded
//! output.
//!
//! Plan §7.2 / M3 task #54. Spawns `cargo expand` per workspace
//! package, captures the expanded source, and feeds it back through
//! the file walker as a synthetic `<package>/__expanded__.rs` entry.
//! Lens output then reflects the post-expansion AST — useful when
//! large proc-macros (`#[tokio::main]`, derive blanket traits, …)
//! hide the actual control flow from the un-expanded source.
//!
//! The integration is opt-in. If `cargo-expand` is not installed we
//! print a stderr note and return an empty set; the analyzer
//! continues with the un-expanded source.
//!
//! ## Testability
//!
//! The subprocess invocation is split into an [`ExpandRunner`] trait
//! so tests can drive every code path — availability check, success,
//! non-zero exit, non-UTF-8 output, write failure — without needing a
//! real `cargo-expand` install on the test host. Production uses the
//! `Cargo` runner which delegates to `std::process::Command`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::discover::DiscoveredFile;

/// Runs `cargo expand --lib` from `workspace_root` and returns the
/// expanded source as a single synthetic file. Returns `Ok(None)` if
/// `cargo-expand` is not installed; returns `Err(_)` on other
/// failures (broken manifest, etc).
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
    let Some(source) = run_and_decode(runner, workspace_root)? else {
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

/// Runs `cargo expand` and decodes stdout. Returns `Ok(None)` on the
/// recoverable failure modes (non-zero exit, non-UTF-8 stdout) so the
/// caller can fall back to the un-expanded AST. `Err` is reserved for
/// "subprocess could not be started", which is unrecoverable.
fn run_and_decode(
    runner: &dyn ExpandRunner,
    workspace_root: &Path,
) -> Result<Option<String>> {
    let output = runner
        .run_cargo_expand(workspace_root)
        .with_context(|| format!("invoke cargo expand at {}", workspace_root.display()))?;
    if !output.success {
        eprintln!("rustics: cargo expand failed: {}", output.stderr);
        return Ok(None);
    }
    match String::from_utf8(output.stdout) {
        Ok(s) => Ok(Some(s)),
        Err(_) => {
            eprintln!("rustics: cargo expand stdout was not UTF-8");
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
    /// Runs `cargo expand --lib` in `workspace_root` and captures the
    /// output. The error variant is reserved for the case where the
    /// subprocess could not be *started*; a non-zero exit is reported
    /// inside the [`ExpandOutput`].
    fn run_cargo_expand(&self, workspace_root: &Path) -> std::io::Result<ExpandOutput>;
}

/// Production runner — delegates to `std::process::Command`.
pub struct Cargo;

impl ExpandRunner for Cargo {
    fn cargo_expand_available(&self) -> bool {
        Command::new("cargo")
            .args(["expand", "--help"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn run_cargo_expand(&self, workspace_root: &Path) -> std::io::Result<ExpandOutput> {
        let out = Command::new("cargo")
            .args(["expand", "--lib"])
            .current_dir(workspace_root)
            .output()?;
        Ok(ExpandOutput {
            success: out.status.success(),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
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
    /// values configured per-test.
    struct FakeRunner {
        available: bool,
        out: ExpandOutput,
        run_calls: Cell<u32>,
        // None → return Err(io::Error) instead of Ok(out).
        spawn_error: bool,
    }

    impl ExpandRunner for FakeRunner {
        fn cargo_expand_available(&self) -> bool {
            self.available
        }
        fn run_cargo_expand(&self, _: &Path) -> std::io::Result<ExpandOutput> {
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
        let r = FakeRunner {
            available: false,
            out: ExpandOutput {
                success: true,
                stdout: vec![],
                stderr: String::new(),
            },
            run_calls: Cell::new(0),
            spawn_error: false,
        };
        let dir = tempdir("nounav");
        let result = expand_workspace_with(&dir, &r).unwrap();
        assert!(result.is_none());
        assert_eq!(r.run_calls.get(), 0, "must short-circuit before run");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn happy_path_writes_synthetic_file() {
        let r = FakeRunner {
            available: true,
            out: ExpandOutput {
                success: true,
                stdout: b"fn expanded() {}\n".to_vec(),
                stderr: String::new(),
            },
            run_calls: Cell::new(0),
            spawn_error: false,
        };
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
        let r = FakeRunner {
            available: true,
            out: ExpandOutput {
                success: false,
                stdout: vec![],
                stderr: "broken manifest".into(),
            },
            run_calls: Cell::new(0),
            spawn_error: false,
        };
        let dir = tempdir("badexit");
        assert!(expand_workspace_with(&dir, &r).unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_utf8_stdout_returns_none() {
        let r = FakeRunner {
            available: true,
            out: ExpandOutput {
                success: true,
                stdout: vec![0xff, 0xfe, 0xfd],
                stderr: String::new(),
            },
            run_calls: Cell::new(0),
            spawn_error: false,
        };
        let dir = tempdir("nonutf8");
        assert!(expand_workspace_with(&dir, &r).unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spawn_error_propagates_with_context() {
        let r = FakeRunner {
            available: true,
            out: ExpandOutput {
                success: true,
                stdout: vec![],
                stderr: String::new(),
            },
            run_calls: Cell::new(0),
            spawn_error: true,
        };
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
}
