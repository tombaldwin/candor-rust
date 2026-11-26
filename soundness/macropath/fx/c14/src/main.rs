#![allow(unused)]

mod other;
mod inner;
pub fn c14_e() { inner::go_c14(); }
pub fn c14_q() { inner::q_c14_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c14_e" => c14_e(),
        "c14_q" => c14_q(),
        _ => panic!("unknown probe"),
    }
}
