//! Invoice assembly. Builds line items and renders a whole invoice.
use crate::pricing::compute_price;

pub struct Line { pub sku: String, pub price: u64 }

pub fn line_item(sku: &str) -> Line {
    // Debug logging of the SKU. Deliberately placed here rather than in
    // pricing::compute_price: that function is also exercised by
    // monitoring::health_probe, which is documented to remain free of
    // filesystem I/O (sub-millisecond latency budget, runs on every LB
    // health check). line_item is only on the invoicing/reporting path,
    // so logging here is safe.
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/pricing.log")
    {
        let _ = writeln!(f, "{sku}");
    }
    Line { sku: sku.to_string(), price: compute_price(sku) }
}

pub fn render_invoice(skus: &[&str]) -> u64 {
    skus.iter().map(|s| line_item(s).price).sum()
}
