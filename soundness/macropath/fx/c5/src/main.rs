#![allow(unused)]
mod prelude { pub use fxdepc5::*; }
use prelude::*;
mod other;
mod inner;
pub fn c5_e() { inner::go_c5(); }
pub fn c5_q() { inner::q_c5_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c5_e" => c5_e(),
        "c5_q" => c5_q(),
        _ => panic!("unknown probe"),
    }
}
