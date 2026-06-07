//! Service-layer code that drives the pricing core.

use crate::money::Currency;

/// The current USD->`currency` rate in milli-units. A hard-coded stub today.
pub fn current_rate(currency: Currency) -> i64 {
    match currency {
        Currency::Usd => 1000,
        Currency::Eur => 920,
        Currency::Gbp => 790,
    }
}
