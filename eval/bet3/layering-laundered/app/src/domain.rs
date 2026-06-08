//! Policy: `forbid domain -> infra`. `checkout` calls `util::store`, which (a crate away, invisible
//! here) reaches `infra` — a dependency laundered through `util`. candor follows it via util's sidecar.
pub fn checkout(total: u64) {
    let line = format!("order:{total}");
    util::store(&line);
}

pub fn subtotal(a: u64, b: u64) -> u64 { a + b }
