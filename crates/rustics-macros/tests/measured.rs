//! `trybuild` end-to-end tests for `#[measured(...)]`.
//!
//! The proc-macro entry point can't be invoked from inside the crate
//! that defines it (proc-macro tokens are only valid during the
//! compilation of *another* crate). `trybuild` is the canonical way
//! to test it: each fixture is compiled as if it were a real
//! downstream crate, and we assert pass/fail via real rustc invocation.
//!
//! Stderr snapshots for the failing fixtures live alongside the
//! corresponding `.rs` files as `<name>.stderr` and are auto-managed
//! by `trybuild`.

#[test]
fn pass_fixtures() {
    let t = trybuild::TestCases::new();
    t.pass("tests/fixtures/pass/*.rs");
}

#[test]
fn fail_fixtures() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/fixtures/fail/*.rs");
}
