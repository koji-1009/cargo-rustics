//! Error path. Designed to fire panic-density.

pub fn require_positive(x: i32) -> i32 {
    let _a = x.checked_abs().unwrap();
    let _b = x.checked_neg().unwrap();
    let _c = x.checked_add(1).unwrap();
    let _d = x.checked_mul(2).unwrap();
    if x < 0 {
        panic!("require_positive: got negative");
    }
    x
}
