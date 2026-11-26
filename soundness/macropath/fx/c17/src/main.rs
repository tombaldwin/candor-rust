#![allow(unused)]

mod other;
mod inner;
pub fn c17_e() { inner::go_c17(); }
pub fn c17_q() { inner::q_c17_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c17_e" => c17_e(),
        "c17_q" => c17_q(),
        _ => panic!("unknown probe"),
    }
}
