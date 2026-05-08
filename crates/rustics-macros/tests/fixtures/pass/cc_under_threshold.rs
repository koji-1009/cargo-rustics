//! `#[measured(cyclomatic_complexity < 10)]` on a tiny function — must compile.

use rustics_macros::measured;

#[measured(cyclomatic_complexity < 10)]
fn small(x: i32) -> i32 {
    if x > 0 { x } else { 0 }
}

fn main() {
    let _ = small(0);
}
