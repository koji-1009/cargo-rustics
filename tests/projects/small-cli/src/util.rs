//! Clean helpers. Should produce *no* violations — it is the control
//! group that proves the analyzer doesn't false-positive on simple code.

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn double(x: i32) -> i32 {
    x * 2
}

pub fn first_or<T: Clone>(slice: &[T], default: T) -> T {
    slice.first().cloned().unwrap_or(default)
}
