#![allow(unused)]
use fxdepc3r::*;
mod other;
mod inner;
pub fn c3r_e() { inner::go_c3r(); }
pub fn c3r_q() { inner::q_c3r_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c3r_e" => c3r_e(),
        "c3r_q" => c3r_q(),
        _ => panic!("unknown probe"),
    }
}
