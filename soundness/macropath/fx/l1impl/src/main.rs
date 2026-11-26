#![allow(unused)]
mod inner; mod defs;
pub fn l1impl_e() { inner::S_l1impl.go_l1impl(); }
pub fn l1impl_q() { inner::SQ_l1impl.qgo_l1impl(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1impl_e" => l1impl_e(),
        "l1impl_q" => l1impl_q(),
        _ => panic!("unknown probe"),
    }
}
