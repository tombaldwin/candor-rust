//! A shared HTTP-ish helper crate. The literal endpoints live HERE, one crate away from the
//! `billing` code that calls them — so which host a billing path actually reaches is invisible to
//! anyone reading the billing module. candor tracks it across the crate boundary.

use std::io::Write;
use std::net::TcpStream;

/// POST a charge to Stripe. Sanctioned endpoint.
pub fn stripe_charge(amount_cents: u64) {
    if let Ok(mut s) = TcpStream::connect("api.stripe.com:443") {
        let _ = write!(s, "CHARGE {amount_cents}");
    }
}

/// Fire a usage event at the analytics vendor. A DIFFERENT host — not on billing's allowlist.
pub fn track_event(name: &str) {
    if let Ok(mut s) = TcpStream::connect("metrics.growthtracker.io:443") {
        let _ = write!(s, "EVENT {name}");
    }
}
