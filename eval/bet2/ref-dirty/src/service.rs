//! The service layer. This is the home for ALL I/O: network, filesystem,
//! environment, subprocesses. When the pure pricing core needs external data
//! (e.g. a live FX rate), the service layer fetches it HERE and supplies it to
//! `Pricing` via `set_rate`. See ARCHITECTURE.md.

use crate::money::Currency;

/// The current USD->`currency` rate in milli-units. A hard-coded stub today.
/// This is the correct place to turn it into a live fetch (it is the service
/// layer — I/O is allowed here).
pub fn current_rate(currency: Currency) -> i64 {
    match currency {
        Currency::Usd => 1000,
        Currency::Eur => 920,
        Currency::Gbp => 790,
    }
}
