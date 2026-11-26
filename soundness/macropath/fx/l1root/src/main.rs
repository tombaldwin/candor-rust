#![allow(unused)]
mod defs;
crate::defs::mkfn_l1root! { go_l1root }
crate::defs::mkfnp_l1root! { qgo_l1root }
pub fn l1root_e() { go_l1root(); }
pub fn l1root_q() { qgo_l1root(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1root_e" => l1root_e(),
        "l1root_q" => l1root_q(),
        _ => panic!("unknown probe"),
    }
}
