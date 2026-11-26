#![allow(unused)]
mod inner;
mod defs;
pub fn c9c_e() { inner::go_c9c(); }
pub fn c9c_q() { inner::q_c9c_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c9c_e" => c9c_e(),
        "c9c_q" => c9c_q(),
        _ => panic!("unknown probe"),
    }
}
