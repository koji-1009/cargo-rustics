//! Several constraints in one invocation — every operator exercised.

use rustics_macros::measured;

#[measured(
    cyclomatic_complexity <= 10,
    cognitive_complexity <= 15,
    lifetime_arity == 0,
    generic_arity != 9999,
    halstead_volume < 1500,
)]
fn ok(x: i32) -> i32 {
    x + 1
}

fn main() {
    let _ = ok(0);
}
