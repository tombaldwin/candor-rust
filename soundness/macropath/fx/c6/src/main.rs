#![allow(unused)]
use fxdepc6::*;
mod other;
mod inner;
pub fn c6_e() { inner::go_c6(); }
pub fn c6_q() { inner::q_c6_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c6_e" => c6_e(),
        "c6_q" => c6_q(),
        _ => panic!("unknown probe"),
    }
}
