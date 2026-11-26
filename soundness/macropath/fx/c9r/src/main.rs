#![allow(unused)]
mod inner;
mod defs;
pub fn c9r_e() { inner::go_c9r(); }
pub fn c9r_q() { inner::q_c9r_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c9r_e" => c9r_e(),
        "c9r_q" => c9r_q(),
        _ => panic!("unknown probe"),
    }
}
