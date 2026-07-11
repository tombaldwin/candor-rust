//! A tiny order-pricing service in three layers: api (allowed to do I/O) → domain (pure business
//! logic — MUST NOT touch the network) → infra (the I/O adapter). The architecture policy is
//! `deny Net domain`. Right now `domain::price_quote` reaches the network transitively (through
//! `infra::fetch_rate`), which VIOLATES the boundary. Fix it so `.candor/policy` passes — the domain
//! must become pure, with the network call performed in the `api` layer and the result threaded down.

pub mod infra {
    /// The real network call: fetch the current FX rate from the pricing service.
    pub fn fetch_rate() -> u64 {
        // (stand-in for a real HTTP GET)
        let _ = std::net::TcpStream::connect("rates.example.com:443");
        100
    }
}

pub mod domain {
    /// Price a single quote. Business logic — should be a PURE function of its inputs.
    pub fn price_quote() -> u64 {
        let rate = crate::infra::fetch_rate();
        rate + 1
    }
    /// Price a bulk order.
    pub fn quote_bulk() -> u64 {
        price_quote() * 2
    }
}

pub mod api {
    /// The API entry point — this layer is allowed to perform I/O.
    pub fn get_quote() -> u64 {
        crate::domain::quote_bulk()
    }
}
