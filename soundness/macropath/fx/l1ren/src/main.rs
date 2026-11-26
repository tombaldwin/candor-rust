#![allow(unused)]
mod inner; mod defs;
use inner::go_l1ren as g_l1ren;
use inner::qgo_l1ren as qg_l1ren;
pub fn l1ren_e() { g_l1ren(); }
pub fn l1ren_q() { qg_l1ren(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1ren_e" => l1ren_e(),
        "l1ren_q" => l1ren_q(),
        _ => panic!("unknown probe"),
    }
}
