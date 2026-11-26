#![allow(unused)]
use fxdepc6r::*;
mod other;
mod inner;
pub fn c6r_e() { inner::go_c6r(); }
pub fn c6r_q() { inner::q_c6r_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c6r_e" => c6r_e(),
        "c6r_q" => c6r_q(),
        _ => panic!("unknown probe"),
    }
}
