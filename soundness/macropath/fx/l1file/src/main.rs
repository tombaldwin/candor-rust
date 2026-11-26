#![allow(unused)]
mod inner; mod defs; mod other;
pub fn l1file_e() { other::via_l1file(); }
pub fn l1file_q() { other::qvia_l1file(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1file_e" => l1file_e(),
        "l1file_q" => l1file_q(),
        _ => panic!("unknown probe"),
    }
}
