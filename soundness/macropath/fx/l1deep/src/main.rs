#![allow(unused)]
mod inner; mod defs;
pub fn l1deep_e() { inner::rel_l1deep(); }
pub fn l1deep_q() { inner::relq_l1deep(); }
pub fn l1deepabs_e() { inner::deep::go_l1deep(); }
pub fn l1deepabs_q() { inner::deep::qgo_l1deep(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1deep_e" => l1deep_e(),
        "l1deep_q" => l1deep_q(),
        "l1deepabs_e" => l1deepabs_e(),
        "l1deepabs_q" => l1deepabs_q(),
        _ => panic!("unknown probe"),
    }
}
