//! The pure domain. Policy: `forbid domain -> infra`. `checkout` reaches into the `infra` CRATE — a
//! dependency that lives across the crate boundary, invisible to a reader of this file's imports list
//! if it were laundered, but here a direct architectural violation candor catches transitively.
pub fn checkout(total: u64) {
    let line = format!("order:{total}");
    persist(&line);
}

fn persist(line: &str) {
    // domain reaching into the infra crate — the forbidden dependency.
    infra::save(line);
}

pub fn subtotal(a: u64, b: u64) -> u64 {
    a + b // pure: depends on nothing
}
