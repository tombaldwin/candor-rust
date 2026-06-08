//! Invoice assembly. Builds line items and renders a whole invoice.
use crate::pricing::compute_price;
use std::fs::OpenOptions;
use std::io::Write;

pub struct Line { pub sku: String, pub price: u64 }

pub fn line_item(sku: &str) -> Line {
    // Debug logging for compute_price, relocated here from `pricing::compute_price`:
    // the health-probe path (monitoring::health_probe -> compute_price) MUST stay free of
    // filesystem I/O (sub-millisecond budget, runs on every health check), so we cannot log
    // inside compute_price itself. line_item is on every other (invoice/report) path.
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("/tmp/pricing.log") {
        let _ = writeln!(f, "{sku}");
    }
    Line { sku: sku.to_string(), price: compute_price(sku) }
}

pub fn render_invoice(skus: &[&str]) -> u64 {
    skus.iter().map(|s| line_item(s).price).sum()
}
