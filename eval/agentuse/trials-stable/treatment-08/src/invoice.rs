//! Invoice assembly. Builds line items and renders a whole invoice.
use crate::pricing::compute_price;

pub struct Line { pub sku: String, pub price: u64 }

pub fn line_item(sku: &str) -> Line {
    // Debug logging of the SKU being priced. Deliberately placed here rather than in
    // pricing::compute_price: compute_price is also reached by monitoring::health_probe, whose
    // documented constraint forbids any filesystem I/O (sub-millisecond, called many times per
    // second). line_item is the invoice-path caller that carries the SKU and is off that hot path.
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
