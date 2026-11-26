#![allow(unused)]
use fxdepc7r::*;
mod other;
mod inner;
pub fn c7r_e() { inner::go_c7r(); }
pub fn c7r_q() { inner::q_c7r_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c7r_e" => c7r_e(),
        "c7r_q" => c7r_q(),
        _ => panic!("unknown probe"),
    }
}
