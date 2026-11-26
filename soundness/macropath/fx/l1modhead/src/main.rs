#![allow(unused)]
mod defs;
defs::mkfn_l1modhead! { go_l1modhead }
defs::mkfnp_l1modhead! { qgo_l1modhead }
pub fn l1modhead_e() { go_l1modhead(); }
pub fn l1modhead_q() { qgo_l1modhead(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1modhead_e" => l1modhead_e(),
        "l1modhead_q" => l1modhead_q(),
        _ => panic!("unknown probe"),
    }
}
