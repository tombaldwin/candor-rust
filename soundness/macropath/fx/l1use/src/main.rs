#![allow(unused)]
mod inner; mod defs;
use inner::{go_l1use, qgo_l1use};
pub fn l1use_e() { go_l1use(); }
pub fn l1use_q() { qgo_l1use(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1use_e" => l1use_e(),
        "l1use_q" => l1use_q(),
        _ => panic!("unknown probe"),
    }
}
