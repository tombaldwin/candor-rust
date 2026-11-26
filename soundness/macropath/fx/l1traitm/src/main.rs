#![allow(unused)]
mod inner; mod defs;
pub fn l1traitm_e() { { use inner::T_l1traitm; inner::S_l1traitm.go_l1traitm() }; }
pub fn l1traitm_q() { { use inner::TQ_l1traitm; inner::SQ_l1traitm.qgo_l1traitm() }; }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1traitm_e" => l1traitm_e(),
        "l1traitm_q" => l1traitm_q(),
        _ => panic!("unknown probe"),
    }
}
