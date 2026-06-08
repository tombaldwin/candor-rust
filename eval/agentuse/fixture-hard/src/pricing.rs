//! Shared pricing helper — the single entry point everything uses to get a taxed amount.
use crate::tax::apply_tax;
pub fn priced(amount: u64) -> u64 { apply_tax(amount) }
