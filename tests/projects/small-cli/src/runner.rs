//! State-machine runner. Designed to fire maximum-nesting-level.

pub fn step(state: u8, input: u8) -> u8 {
    let mut next = state;
    if state == 0 {
        if input == b'A' {
            for _ in 0..2 {
                if input.is_ascii_alphabetic() {
                    if input == b'A' {
                        next = 1;
                    }
                }
            }
        }
    }
    next
}
