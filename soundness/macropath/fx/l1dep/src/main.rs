#![allow(unused)]
mod inner;
pub fn l1dep_e() { inner::p_inner_l1dep(); }
pub fn l1dep_q() { inner::q_inner_l1dep(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1dep_e" => l1dep_e(),
        "l1dep_q" => l1dep_q(),
        _ => panic!("unknown probe"),
    }
}
