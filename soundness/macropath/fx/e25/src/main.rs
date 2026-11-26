#![allow(unused)]
mod prelude { pub use fxdepe25::*; }
pub use inner::gated_e25;
mod inner;
pub fn e25_e() { inner::p_go_e25(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e25_e" => e25_e(),
        "gated_e25" => gated_e25(),
        _ => panic!("unknown probe"),
    }
}
