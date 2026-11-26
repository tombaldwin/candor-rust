#![allow(unused)]
mod inner;
mod defs;
pub fn c9_e() { inner::go_c9(); }
pub fn c9_q() { inner::q_c9_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c9_e" => c9_e(),
        "c9_q" => c9_q(),
        _ => panic!("unknown probe"),
    }
}
