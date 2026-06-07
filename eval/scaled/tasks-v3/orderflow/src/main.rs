//! Wire the app together and exercise each entry point.

mod admin;
mod api;
mod cart;
mod catalog;
mod checkout;
mod discount;
mod money;
mod pricing;
mod report;
mod service;
mod tax;

use cart::Cart;
use checkout::Checkout;
use discount::Discount;
use money::Currency;
use pricing::Pricing;
use service::OrderService;
use tax::Tax;

fn main() {
    let pricing = Pricing::new();
    let checkout = Checkout::new(Tax::new(20), Discount::new(5000, 10));
    let service = OrderService::new(pricing, checkout);

    let skus = ["abcd", "ef", "ghijk", "lmn"];

    // API surface.
    println!("{}", api::get_quote(&service, "abcd", Currency::Eur));
    println!("{}", api::list_quotes(&service, &skus, Currency::Gbp));

    let mut cart = Cart::new(Currency::Usd);
    cart.add("abcd", 2);
    cart.add("ghijk", 1);
    println!("{}", api::post_checkout(&service, &cart));

    // Periodic dashboard + admin tooling.
    println!("revenue = {}", report::daily_revenue(&service, &skus));
    println!("prices = {:?}", admin::recompute_prices(service.pricing_ref(), &skus));
}
