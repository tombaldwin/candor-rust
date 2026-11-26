#![allow(unused)]

mod defs;
mod other;
mod inner;
pub fn flimnest_e() { inner::go_flimnest(); }
pub fn flimnest_q() { inner::q_flimnest_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "flimnest_e" => flimnest_e(),
        "flimnest_q" => flimnest_q(),
        _ => panic!("unknown probe"),
    }
}
