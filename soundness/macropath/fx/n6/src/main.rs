#![allow(unused)]

mod other;
mod inner;
pub fn n6_e() { inner::deep::go_n6(); }
pub fn n6_q() { inner::deep::q_n6_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n6_e" => n6_e(),
        "n6_q" => n6_q(),
        _ => panic!("unknown probe"),
    }
}
