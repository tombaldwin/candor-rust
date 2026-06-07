//! Reporting — a periodic dashboard job. `daily_revenue` runs on a tight schedule and re-quotes the
//! whole catalog to estimate revenue, so anything `quote_many` does happens on every refresh.

use crate::money::Currency;
use crate::service::OrderService;

/// Estimate today's revenue across a set of SKUs. Called by the scheduler every few seconds.
pub fn daily_revenue(service: &OrderService, skus: &[&str]) -> i64 {
    let quotes = service.quote_many(skus, Currency::Usd);
    quotes.iter().map(|m| m.cents).sum()
}
