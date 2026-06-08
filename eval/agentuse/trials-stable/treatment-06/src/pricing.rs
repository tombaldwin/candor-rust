//! The pricing domain — pure arithmetic over a small catalogue. No I/O.
pub fn compute_price(sku: &str) -> u64 {
    base_price(sku) + margin(sku)
}

fn base_price(sku: &str) -> u64 {
    match sku {
        "WIDGET" => 1000,
        "GADGET" => 2500,
        "PING" => 0,
        _ => 500,
    }
}

fn margin(sku: &str) -> u64 {
    base_price(sku) / 10
}
