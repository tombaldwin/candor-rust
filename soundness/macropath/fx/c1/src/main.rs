#![allow(unused)]

mod other;
mod inner;
pub fn c1_e() { inner::go_c1(); }
pub fn c1_q() { inner::q_c1_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c1_e" => c1_e(),
        "c1_q" => c1_q(),
        _ => panic!("unknown probe"),
    }
}
