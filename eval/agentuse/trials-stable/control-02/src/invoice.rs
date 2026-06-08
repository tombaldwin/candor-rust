//! Invoice assembly. Builds line items and renders a whole invoice.
use crate::pricing::compute_price;
use std::fs::OpenOptions;
use std::io::Write;

pub struct Line { pub sku: String, pub price: u64 }

pub fn line_item(sku: &str) -> Line {
    // Debug logging of the SKU. Placed here rather than in pricing::compute_price
    // because compute_price is also exercised by monitoring::health_probe, which is
    // documented to remain free of filesystem I/O. line_item is on the invoice/report
    // path only and never feeds the health probe, so logging here is safe.
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("/tmp/pricing.log") {
        let _ = writeln!(f, "{sku}");
    }
    Line { sku: sku.to_string(), price: compute_price(sku) }
}

pub fn render_invoice(skus: &[&str]) -> u64 {
    skus.iter().map(|s| line_item(s).price).sum()
}
