#![allow(unused)]
use fxdepc2::*;
mod other;
mod inner;
pub fn c2_e() { inner::go_c2(); }
pub fn c2_q() { inner::q_c2_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c2_e" => c2_e(),
        "c2_q" => c2_q(),
        _ => panic!("unknown probe"),
    }
}
