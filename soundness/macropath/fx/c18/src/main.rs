#![allow(unused)]
#[macro_use] extern crate fxdepc18;
use fxdepc18::*;
mod other;
mod inner;
pub fn c18_e() { inner::go_c18(); }
pub fn c18_q() { inner::q_c18_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c18_e" => c18_e(),
        "c18_q" => c18_q(),
        _ => panic!("unknown probe"),
    }
}
