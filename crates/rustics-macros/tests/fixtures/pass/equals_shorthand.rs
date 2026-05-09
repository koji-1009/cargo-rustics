//! `metric = N` desugars to `metric <= N` — the dominant max-threshold
//! shape. CC of this function is 1, comfortably below 10.

use rustics_macros::measured;

#[measured(cyclomatic_complexity = 10)]
fn ok() -> i32 {
    1
}

fn main() {
    let _ = ok();
}
