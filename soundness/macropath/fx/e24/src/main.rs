#![allow(unused)]

pub use inner::gated_e24;
mod prelude;
mod inner;
pub fn e24_e() { inner::p_go_e24(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e24_e" => e24_e(),
        "gated_e24" => gated_e24(),
        _ => panic!("unknown probe"),
    }
}
