#![allow(unused)]
mod inner; mod defs;
use inner::*;
pub fn l1glob_e() { go_l1glob(); }
pub fn l1glob_q() { qgo_l1glob(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1glob_e" => l1glob_e(),
        "l1glob_q" => l1glob_q(),
        _ => panic!("unknown probe"),
    }
}
