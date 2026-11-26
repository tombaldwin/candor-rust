#![allow(unused)]
mod inner; mod defs;
pub fn l1ufcs_e() { inner::S_l1ufcs::go_l1ufcs(&inner::S_l1ufcs); }
pub fn l1ufcs_q() { inner::SQ_l1ufcs::qgo_l1ufcs(&inner::SQ_l1ufcs); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1ufcs_e" => l1ufcs_e(),
        "l1ufcs_q" => l1ufcs_q(),
        _ => panic!("unknown probe"),
    }
}
