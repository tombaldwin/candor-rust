//! Invoice assembly. Builds line items and renders a whole invoice.
use crate::pricing::compute_price;

pub struct Line { pub sku: String, pub price: u64 }

pub fn line_item(sku: &str) -> Line {
    Line { sku: sku.to_string(), price: compute_price(sku) }
}

pub fn render_invoice(skus: &[&str]) -> u64 {
    skus.iter().map(|s| line_item(s).price).sum()
}
