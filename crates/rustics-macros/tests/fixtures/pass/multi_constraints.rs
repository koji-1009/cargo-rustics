//! Several constraints in one invocation — every operator exercised.

use rustics_macros::measured;

#[measured(
    cyclomatic_complexity <= 10,
    number_of_parameters <= 5,
    lifetime_arity == 0,
    generic_arity != 9999,
    method_length < 100,
)]
fn ok(x: i32) -> i32 {
    x + 1
}

fn main() {
    let _ = ok(0);
}
