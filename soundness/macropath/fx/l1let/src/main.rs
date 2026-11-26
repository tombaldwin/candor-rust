#![allow(unused)]
mod inner; mod defs;
pub fn l1let_q() { inner::p_inner_l1let(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1let_q" => l1let_q(),
        _ => panic!("unknown probe"),
    }
}
