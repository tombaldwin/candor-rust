//! Checkout — assembles the final amount a customer pays (total − discount) and "places" the order.

use crate::cart::Cart;
use crate::discount::Discount;
use crate::money::Money;
use crate::pricing::Pricing;
use crate::tax::Tax;

pub struct Checkout {
    pub tax: Tax,
    pub discount: Discount,
}

impl Checkout {
    pub fn new(tax: Tax, discount: Discount) -> Self {
        Checkout { tax, discount }
    }

    /// Review the cart: the payable amount after tax and discount. (No side effects beyond pricing.)
    pub fn review(&self, pricing: &Pricing, cart: &Cart) -> Money {
        let total = cart.total(pricing, &self.tax);
        let off = self.discount.for_cart(pricing, cart);
        Money::new(total.cents - off.cents, total.currency)
    }

    /// Place the order, returning the charged amount. (In a real system this also writes the order;
    /// here it just computes what would be charged.)
    pub fn place(&self, pricing: &Pricing, cart: &Cart) -> Money {
        let payable = self.review(pricing, cart);
        payable
    }
}
