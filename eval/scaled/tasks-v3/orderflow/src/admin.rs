//! Admin tooling — back-office helpers an operator runs by hand.

use crate::money::Currency;
use crate::pricing::Pricing;

/// Recompute and print the current price of each SKU (e.g. after a catalog change).
pub fn recompute_prices(pricing: &Pricing, skus: &[&str]) -> Vec<i64> {
    skus.iter()
        .map(|s| pricing.quote(s, Currency::Usd).cents)
        .collect()
}
