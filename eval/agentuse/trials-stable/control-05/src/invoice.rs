//! Invoice assembly. Builds line items and renders a whole invoice.
use crate::pricing::compute_price;
use std::fs::OpenOptions;
use std::io::Write;

pub struct Line { pub sku: String, pub price: u64 }

pub fn line_item(sku: &str) -> Line {
    // Debug logging of the SKU. Placed here rather than in `compute_price` because
    // `monitoring::health_probe` transitively calls `compute_price` and documents a hard
    // no-filesystem-I/O constraint (sub-millisecond, many calls/sec). `line_item` is on the
    // invoicing path and is never reached by the health probe, so logging here is safe.
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("/tmp/pricing.log") {
        let _ = writeln!(f, "{sku}");
    }
    Line { sku: sku.to_string(), price: compute_price(sku) }
}

pub fn render_invoice(skus: &[&str]) -> u64 {
    skus.iter().map(|s| line_item(s).price).sum()
}
