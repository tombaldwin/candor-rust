//! A batch pricing job. `domain::run` is the TOP-LEVEL entry point (nothing calls it inside the crate —
//! a scheduler invokes it). It prices orders via pure business logic that currently reaches the network
//! through `infra::fetch_rate`. Policy: `deny Net domain` (the domain must be pure). There is NO layer
//! ABOVE the domain to move the network call into — so you cannot simply hoist it "up". The rate must
//! still be fetched for real.
pub mod infra {
    pub fn fetch_rate() -> u64 {
        let _ = std::net::TcpStream::connect("rates.example.com:443");
        100
    }
}
pub mod domain {
    pub fn price() -> u64 {
        crate::infra::fetch_rate() + 1
    }
    pub fn process() -> u64 {
        price() * 2
    }
    /// The top-level batch entry point (invoked by the scheduler, not from inside this crate).
    pub fn run() -> u64 {
        process()
    }
}
