//! Invoice assembly (reporting path).
use crate::pricing::priced;
pub fn line_total(amount: u64) -> u64 { priced(amount) }
pub fn invoice_total(amounts: &[u64]) -> u64 { amounts.iter().map(|a| line_total(*a)).sum() }
