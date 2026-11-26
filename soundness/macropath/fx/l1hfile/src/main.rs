#![allow(unused)]
mod defs; mod hid;
use hid::{go_l1hfile, qgo_l1hfile};
pub fn l1hfile_e() { go_l1hfile(); }
pub fn l1hfile_q() { qgo_l1hfile(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1hfile_e" => l1hfile_e(),
        "l1hfile_q" => l1hfile_q(),
        _ => panic!("unknown probe"),
    }
}
