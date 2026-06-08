//! Invoice assembly. Builds line items and renders a whole invoice.
use crate::pricing::compute_price;
use std::fs::OpenOptions;
use std::io::Write;

pub struct Line { pub sku: String, pub price: u64 }

/// Debug logging lives here (not in `pricing::compute_price`) on purpose: `compute_price` is also on
/// `monitoring::health_probe`'s hot path, which is documented to stay free of filesystem I/O. The
/// invoice path is the safe place to record which SKUs we price.
fn log_sku(sku: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("/tmp/pricing.log") {
        let _ = writeln!(f, "{sku}");
    }
}

pub fn line_item(sku: &str) -> Line {
    log_sku(sku);
    Line { sku: sku.to_string(), price: compute_price(sku) }
}

pub fn render_invoice(skus: &[&str]) -> u64 {
    skus.iter().map(|s| line_item(s).price).sum()
}
