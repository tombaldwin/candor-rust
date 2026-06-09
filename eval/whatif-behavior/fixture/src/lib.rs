//! A small order/pricing crate. `pricing::quote` is the one place a price is produced; it is about to be
//! changed to fetch a live FX rate over TCP. The question both arms answer: which functions transitively
//! gain the Net effect?  (= every transitive caller of quote, plus quote.)
pub mod pricing {
    pub fn quote(cents: u64) -> u64 { cents * 100 }                 // <-- about to open a TcpStream
    pub fn quote_bulk(cents: u64, n: u64) -> u64 { quote(cents) * n }
    pub fn line_item(cents: u64) -> u64 { quote(cents) + 5 }
}
pub mod cart {
    use crate::pricing;
    pub fn total(items: &[u64]) -> u64 { items.iter().map(|c| pricing::quote_bulk(*c, 1)).sum() }
    pub fn subtotal(items: &[u64]) -> u64 { items.iter().map(|c| pricing::line_item(*c)).sum() }
}
pub mod discount {
    use crate::cart;
    pub fn apply(items: &[u64]) -> u64 { cart::total(items) * 9 / 10 }
}
pub mod checkout {
    use crate::{cart, discount};
    pub fn run(items: &[u64]) -> u64 { cart::total(items).min(discount::apply(items)) }
}
pub mod service {
    use crate::checkout;
    pub fn place_order(items: &[u64]) -> u64 { checkout::run(items) }
}
pub mod api {
    use crate::service;
    pub fn handle(items: &[u64]) -> u64 { service::place_order(items) }
}
pub mod report {
    use crate::{cart, service};
    pub fn summary(items: &[u64]) -> u64 { service::place_order(items) + cart::subtotal(items) }
}
pub mod admin {
    use crate::report;
    pub fn audit(items: &[u64]) -> u64 { report::summary(items) }
}
pub fn main_run() { let items = [1u64, 2, 3]; let _ = api::handle(&items); let _ = admin::audit(&items); }
