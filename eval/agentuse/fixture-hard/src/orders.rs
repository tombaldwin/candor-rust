use crate::invoice::invoice_total;
pub fn order_total(order: &[u64]) -> u64 { invoice_total(order) }
