#![allow(unused)]
use fxdepc8::*;
mod other;
mod inner;
pub fn c8_e() { self::mk!(); }
pub fn c8_q() { self::mknop!(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c8_e" => c8_e(),
        "c8_q" => c8_q(),
        _ => panic!("unknown probe"),
    }
}
