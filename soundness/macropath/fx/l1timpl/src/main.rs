#![allow(unused)]
mod inner; mod defs;
pub fn l1timpl_e() { { use inner::Tr_l1timpl; inner::S_l1timpl.go_l1timpl() }; }
pub fn l1timpl_q() { { use inner::TrQ_l1timpl; inner::SQ_l1timpl.qgo_l1timpl() }; }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1timpl_e" => l1timpl_e(),
        "l1timpl_q" => l1timpl_q(),
        _ => panic!("unknown probe"),
    }
}
