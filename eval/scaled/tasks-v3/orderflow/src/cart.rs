//! A shopping cart — lines of (sku, qty). Totals are computed against live pricing + tax.

use crate::money::{Currency, Money};
use crate::pricing::Pricing;
use crate::tax::Tax;

pub struct Line {
    pub sku: String,
    pub qty: i64,
}

pub struct Cart {
    pub lines: Vec<Line>,
    pub currency: Currency,
}

impl Cart {
    pub fn new(currency: Currency) -> Self {
        Cart {
            lines: Vec::new(),
            currency,
        }
    }

    pub fn add(&mut self, sku: &str, qty: i64) {
        self.lines.push(Line {
            sku: sku.to_string(),
            qty,
        });
    }

    /// The price of one line: unit quote × quantity.
    pub fn line_total(&self, pricing: &Pricing, line: &Line) -> Money {
        let unit = pricing.quote(&line.sku, self.currency);
        unit.scale_percent(line.qty * 100)
    }

    /// The pre-tax sum of every line.
    pub fn subtotal(&self, pricing: &Pricing) -> Money {
        let mut total = Money::new(0, self.currency);
        for line in &self.lines {
            total = total.add(self.line_total(pricing, line));
        }
        total
    }

    /// The grand total: subtotal plus tax on the subtotal.
    pub fn total(&self, pricing: &Pricing, tax: &Tax) -> Money {
        let sub = self.subtotal(pricing);
        sub.add(tax.apply(sub))
    }
}
