#![allow(unused)]
extern crate fxdepc16;
mod other;
mod inner;
pub fn c16_e() { inner::go_c16(); }
pub fn c16_q() { inner::q_c16_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c16_e" => c16_e(),
        "c16_q" => c16_q(),
        _ => panic!("unknown probe"),
    }
}
