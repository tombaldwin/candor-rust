#![allow(unused)]
mod inner; mod defs;
pub fn l1collide_e() { inner::p_inner_l1collide(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1collide_e" => l1collide_e(),
        _ => panic!("unknown probe"),
    }
}
