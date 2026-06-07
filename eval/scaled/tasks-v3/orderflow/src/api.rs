//! The HTTP-facing API handlers (stubbed as plain functions). Each turns a request into a service
//! call and formats the result.

use crate::cart::Cart;
use crate::money::Currency;
use crate::service::OrderService;

/// GET /quote?sku=&ccy= — a single price.
pub fn get_quote(service: &OrderService, sku: &str, currency: Currency) -> String {
    let m = service.quote_one(sku, currency);
    format!("{} {:?}", m.cents, m.currency)
}

/// GET /quotes?skus=&ccy= — a price list.
pub fn list_quotes(service: &OrderService, skus: &[&str], currency: Currency) -> String {
    let quotes = service.quote_many(skus, currency);
    let mut out = String::new();
    for q in quotes {
        out.push_str(&format!("{} {:?}\n", q.cents, q.currency));
    }
    out
}

/// POST /checkout — charge a cart.
pub fn post_checkout(service: &OrderService, cart: &Cart) -> String {
    let charged = service.checkout(cart);
    format!("charged {} {:?}", charged.cents, charged.currency)
}
