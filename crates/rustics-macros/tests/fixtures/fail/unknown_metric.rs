//! Unknown metric id — must fail compilation.

use rustics_macros::measured;

#[measured(does_not_exist < 10)]
fn f() {}

fn main() {
    f();
}
