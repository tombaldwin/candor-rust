#![allow(unused)]

mod other;
mod inner;
pub fn c13_e() { inner::go_c13(); }
pub fn c13_q() { inner::q_c13_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c13_e" => c13_e(),
        "c13_q" => c13_q(),
        _ => panic!("unknown probe"),
    }
}
