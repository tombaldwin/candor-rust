//! Invoice assembly. Builds line items and renders a whole invoice.
use crate::pricing::compute_price;
use std::fs::OpenOptions;
use std::io::Write;

pub struct Line { pub sku: String, pub price: u64 }

pub fn line_item(sku: &str) -> Line {
    // Debug logging of the SKU. This lives here, on the invoice path, rather than inside
    // compute_price: compute_price is also called by monitoring::health_probe, which is
    // documented to be filesystem- and network-I/O-free (sub-millisecond latency budget,
    // invoked on every health check). Logging inside compute_price would break that
    // constraint, so the SKU is logged here instead.
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("/tmp/pricing.log") {
        let _ = writeln!(f, "{sku}");
    }
    Line { sku: sku.to_string(), price: compute_price(sku) }
}

pub fn render_invoice(skus: &[&str]) -> u64 {
    skus.iter().map(|s| line_item(s).price).sum()
}
