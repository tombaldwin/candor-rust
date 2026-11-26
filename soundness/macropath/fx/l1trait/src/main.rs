#![allow(unused)]
mod inner; mod defs;
pub fn l1trait_e() { <inner::S_l1trait as inner::T_l1trait>::go_l1trait(&inner::S_l1trait); }
pub fn l1trait_q() { <inner::SQ_l1trait as inner::TQ_l1trait>::qgo_l1trait(&inner::SQ_l1trait); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1trait_e" => l1trait_e(),
        "l1trait_q" => l1trait_q(),
        _ => panic!("unknown probe"),
    }
}
