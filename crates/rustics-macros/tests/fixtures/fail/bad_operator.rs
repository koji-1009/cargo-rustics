//! Unrecognised operator — `Parse` returns the "expected one of …"
//! error.

use rustics_macros::measured;

#[measured(cyclomatic_complexity ?? 10)]
fn f() {}

fn main() {
    f();
}
