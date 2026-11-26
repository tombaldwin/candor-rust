#![allow(unused)]
#[cfg(unix)] #[macro_use] extern crate fxdepe30;
pub use inner::gated_e30;
mod inner;
pub fn e30_e() { inner::p_go_e30(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e30_e" => e30_e(),
        "gated_e30" => gated_e30(),
        _ => panic!("unknown probe"),
    }
}
