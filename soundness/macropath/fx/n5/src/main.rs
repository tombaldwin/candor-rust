#![allow(unused)]

mod other;
mod inner;
pub fn n5_e() { inner::go_n5(); }
pub fn n5_q() { inner::q_n5_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n5_e" => n5_e(),
        "n5_q" => n5_q(),
        _ => panic!("unknown probe"),
    }
}
