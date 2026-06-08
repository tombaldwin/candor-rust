//! Liveness monitoring.
use crate::pricing::compute_price;

/// Load-balancer health probe. Called on EVERY health check (many per second), with a sub-millisecond
/// budget, so it MUST remain free of filesystem and network I/O — it only exercises the pure pricing
/// path as a cheap "is the service computing?" signal. If anything it transitively calls starts doing
/// I/O, the probe blows its latency budget and the LB marks the node unhealthy.
pub fn health_probe() -> bool {
    compute_price("PING") == 0
}
