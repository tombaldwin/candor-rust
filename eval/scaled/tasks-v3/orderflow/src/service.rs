//! The order service — the application-facing layer that owns pricing/checkout and exposes the
//! operations the API and reports call.

use crate::cart::Cart;
use crate::checkout::Checkout;
use crate::money::{Currency, Money};
use crate::pricing::Pricing;

pub struct OrderService {
    pricing: Pricing,
    checkout: Checkout,
}

impl OrderService {
    pub fn new(pricing: Pricing, checkout: Checkout) -> Self {
        OrderService { pricing, checkout }
    }

    /// Quote a single SKU in a currency.
    pub fn quote_one(&self, sku: &str, currency: Currency) -> Money {
        self.pricing.quote(sku, currency)
    }

    /// Quote many SKUs in a currency.
    pub fn quote_many(&self, skus: &[&str], currency: Currency) -> Vec<Money> {
        self.pricing.quote_bulk(skus, currency)
    }

    /// Run a cart through checkout and return the charged amount.
    pub fn checkout(&self, cart: &Cart) -> Money {
        self.checkout.place(&self.pricing, cart)
    }

    /// Borrow the pricing engine (used by back-office tooling).
    pub fn pricing_ref(&self) -> &Pricing {
        &self.pricing
    }
}
