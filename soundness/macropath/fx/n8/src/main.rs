#![allow(unused)]
use fxdepn8::*;
mod m_n8 { pub fn go() { super::mk!() } pub fn goq() { super::mknop!() } }
mod other;
mod inner;
pub fn n8_e() { m_n8::go(); }
pub fn n8_q() { m_n8::goq(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n8_e" => n8_e(),
        "n8_q" => n8_q(),
        _ => panic!("unknown probe"),
    }
}
