//! Pricing — turns a catalog base price into a quote in the requested currency.
//!
//! Today the FX rate is a hardcoded placeholder. `quote` is the one place a price in a foreign
//! currency is produced, so every part of the app that shows a price flows through here.

use crate::catalog::Catalog;
use crate::money::{Currency, Money};

pub struct Pricing {
    catalog: Catalog,
}

impl Pricing {
    pub fn new() -> Self {
        Pricing {
            catalog: Catalog::new(),
        }
    }

    /// Quote `sku` in `currency`. Looks up the base (USD) price and converts it.
    pub fn quote(&self, sku: &str, currency: Currency) -> Money {
        let base = self.catalog.base_price(sku);
        // FX rate × 1000. Placeholder: parity for USD, fixed rates otherwise.
        let rate_milli = match currency {
            Currency::Usd => 1000,
            Currency::Eur => 920,
            Currency::Gbp => 790,
        };
        base.convert(rate_milli, currency)
    }

    /// Quote several SKUs in one currency.
    pub fn quote_bulk(&self, skus: &[&str], currency: Currency) -> Vec<Money> {
        skus.iter().map(|s| self.quote(s, currency)).collect()
    }
}
