#![allow(unused)]
mod inner; mod defs; mod plain;
pub fn l1other_e() { plain::call_l1other(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1other_e" => l1other_e(),
        _ => panic!("unknown probe"),
    }
}
