//! The product catalog — an in-memory base-price book in USD. Pure.

use crate::money::{Currency, Money};

pub struct Catalog;

impl Catalog {
    pub fn new() -> Self {
        Catalog
    }

    /// The list price of a SKU, in USD. A toy deterministic price book.
    pub fn base_price(&self, sku: &str) -> Money {
        let cents = match sku.len() % 5 {
            0 => 1999,
            1 => 2999,
            2 => 4999,
            3 => 999,
            _ => 1499,
        };
        Money::new(cents, Currency::Usd)
    }
}
