//! Invoice assembly. Builds line items and renders a whole invoice.
use crate::pricing::compute_price;

pub struct Line { pub sku: String, pub price: u64 }

pub fn line_item(sku: &str) -> Line {
    // Debug logging of the SKU. Deliberately placed here rather than in
    // pricing::compute_price: the load-balancer health probe
    // (monitoring::health_probe) calls compute_price directly and is
    // documented to stay free of filesystem I/O. All real pricing usage
    // flows through line_item, but the probe does not, so logging here
    // does not violate that constraint.
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
