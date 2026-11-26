#![allow(unused)]

mod other;
mod inner;
pub fn n5r_e() { inner::go_n5r(); }
pub fn n5r_q() { inner::q_n5r_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n5r_e" => n5r_e(),
        "n5r_q" => n5r_q(),
        _ => panic!("unknown probe"),
    }
}
