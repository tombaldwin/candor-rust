// API surface (set_rate, service::current_rate) is intentionally present ahead of being wired up.
#![allow(dead_code)]

mod money;
mod pricing;
mod service;

use money::Currency;
use pricing::Pricing;

fn main() {
    let pricing = Pricing::new();
    for (sku, ccy) in [
        ("WIDGET", Currency::Eur),
        ("GADGET", Currency::Gbp),
        ("GIZMO", Currency::Usd),
    ] {
        let m = pricing.quote(sku, ccy);
        println!("{sku} -> {} {}", m.minor, m.currency.code());
    }
}
