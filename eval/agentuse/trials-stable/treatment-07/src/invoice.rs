//! Invoice assembly. Builds line items and renders a whole invoice.
use crate::pricing::compute_price;

pub struct Line { pub sku: String, pub price: u64 }

pub fn line_item(sku: &str) -> Line {
    // Debug logging of the SKU. Deliberately placed here rather than in
    // pricing::compute_price: compute_price is also on the hot, I/O-free path of
    // monitoring::health_probe (sub-millisecond LB latency budget), so adding a
    // filesystem effect there would break that documented constraint. line_item
    // is only reached by the invoice/report flows, never by the health probe.
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
