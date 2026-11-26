#![allow(unused)]
mod inner; mod defs;
use inner::{go_l1clos, qgo_l1clos};
pub fn l1clos_e() { (|| go_l1clos())(); }
pub fn l1clos_q() { (|| qgo_l1clos())(); }
pub fn l1fnref_e() { { let f: fn() = go_l1clos; f() }; }
pub fn l1fnref_q() { { let f: fn() = qgo_l1clos; f() }; }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1clos_e" => l1clos_e(),
        "l1clos_q" => l1clos_q(),
        "l1fnref_e" => l1fnref_e(),
        "l1fnref_q" => l1fnref_q(),
        _ => panic!("unknown probe"),
    }
}
