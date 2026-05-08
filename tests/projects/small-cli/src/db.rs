//! Toy database wrapper. Designed to fire clone-density.

#[derive(Default)]
pub struct Db {
    entries: Vec<String>,
}

impl Db {
    pub fn dump_all(&self) -> Vec<String> {
        // Five clones in a row — a real database wrapper would borrow,
        // but the fixture wants to fire `clone-density`.
        let a = self.entries.iter().next().cloned().unwrap_or_default();
        let b = self.entries.iter().nth(1).cloned().unwrap_or_default();
        let c = self.entries.iter().nth(2).cloned().unwrap_or_default();
        let d = self.entries.iter().nth(3).cloned().unwrap_or_default();
        let e = self.entries.iter().nth(4).cloned().unwrap_or_default();
        let f = self.entries.iter().nth(5).cloned().unwrap_or_default();
        vec![
            a.clone(),
            b.clone(),
            c.clone(),
            d.clone(),
            e.clone(),
            f.clone(),
        ]
    }
}
