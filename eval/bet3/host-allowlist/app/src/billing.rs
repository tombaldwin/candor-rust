//! The billing module. Policy: `allow Net in billing api.stripe.com hooks.stripe.com` — billing may
//! reach the network, but ONLY Stripe. Nothing in this file names a host: the endpoints are inside
//! `httpkit`, a crate away. A model editing this module cannot see which host each call ends up at;
//! candor resolves it transitively across the crate boundary (AS-EFF-008).

/// Charge a customer via Stripe. Reaches api.stripe.com (allowed).
pub fn charge_customer(cents: u64) {
    httpkit::stripe_charge(cents);
}

/// Record a customer activity event. Looks innocent — but `track_event` reaches
/// metrics.growthtracker.io, which is NOT on billing's allowlist. This is the boundary violation a
/// local read misses and candor catches.
pub fn record_activity(event: &str) {
    httpkit::track_event(event);
}
