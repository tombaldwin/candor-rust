#![allow(unused)]
mod m_n7 { use fxdepn7::{mk, nop as mknop}; pub fn go() { self::mk!() } pub fn goq() { self::mknop!() } }
mod other;
mod inner;
pub fn n7_e() { m_n7::go(); }
pub fn n7_q() { m_n7::goq(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n7_e" => n7_e(),
        "n7_q" => n7_q(),
        _ => panic!("unknown probe"),
    }
}
