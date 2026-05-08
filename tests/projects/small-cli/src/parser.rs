//! Hand-rolled parser. Designed to violate cyclomatic-complexity AND
//! maximum-nesting-level — a single deeply-nested if/else chain.

pub struct Parser;

impl Parser {
    pub fn parse(&self, input: &str) -> Result<i32, ()> {
        let bytes = input.as_bytes();
        if bytes.is_empty() {
            return Err(());
        }
        if bytes[0] == b'-' {
            if bytes.len() < 2 {
                return Err(());
            }
            if bytes.get(1) == Some(&b'0') {
                return Ok(0);
            } else if bytes.get(1) == Some(&b'1') {
                return Ok(-1);
            } else if bytes.get(1) == Some(&b'2') {
                return Ok(-2);
            } else if bytes.get(1) == Some(&b'3') {
                return Ok(-3);
            } else if bytes.get(1) == Some(&b'4') {
                return Ok(-4);
            } else {
                return Err(());
            }
        }
        match bytes[0] {
            b'0' => Ok(0),
            b'1' => Ok(1),
            b'2' => Ok(2),
            b'3' => Ok(3),
            b'4' => Ok(4),
            b'5' => Ok(5),
            b'6' => Ok(6),
            b'7' => Ok(7),
            b'8' => Ok(8),
            b'9' => Ok(9),
            _ => Err(()),
        }
    }
}
