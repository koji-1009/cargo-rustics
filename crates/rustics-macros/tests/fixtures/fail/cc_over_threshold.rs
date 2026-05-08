//! Crossing the CC threshold — must fail compilation with the macro's
//! `compile_error!(...)` message.

use rustics_macros::measured;

#[measured(cyclomatic_complexity < 2)]
fn heavy(x: i32, y: i32) -> i32 {
    let mut a = 0;
    if x > 0 { a += 1; }
    if y > 0 { a += 1; }
    if x + y > 0 { a += 1; }
    a
}

fn main() {
    let _ = heavy(1, 2);
}
