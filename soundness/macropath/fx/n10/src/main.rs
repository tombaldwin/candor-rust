#![allow(unused)]
pub fn host_n10() { mod m { use fxdepn10::{mk, nop as mknop}; pub fn g() { self::mk!() } pub fn gq() { self::mknop!() } } m::g() }
pub fn hostq_n10() { mod m { use fxdepn10::{mk, nop as mknop}; pub fn gq() { self::mknop!() } } m::gq() }
mod other;
mod inner;
pub fn n10_e() { host_n10(); }
pub fn n10_q() { hostq_n10(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n10_e" => n10_e(),
        "n10_q" => n10_q(),
        _ => panic!("unknown probe"),
    }
}
