//! Discounts — a simple "spend over a threshold, take a percentage off" rule, evaluated against the
//! cart's live subtotal.

use crate::cart::Cart;
use crate::money::Money;
use crate::pricing::Pricing;

pub struct Discount {
    threshold_cents: i64,
    pct_off: i64,
}

impl Discount {
    pub fn new(threshold_cents: i64, pct_off: i64) -> Self {
        Discount {
            threshold_cents,
            pct_off,
        }
    }

    /// The discount amount this cart qualifies for (zero if under the threshold).
    pub fn for_cart(&self, pricing: &Pricing, cart: &Cart) -> Money {
        let sub = cart.subtotal(pricing);
        if sub.cents >= self.threshold_cents {
            sub.scale_percent(self.pct_off)
        } else {
            Money::new(0, sub.currency)
        }
    }
}
