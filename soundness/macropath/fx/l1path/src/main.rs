#![allow(unused)]
mod inner; mod defs;
pub fn l1path_e() { inner::go_l1path(); }
pub fn l1path_q() { inner::qgo_l1path(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1path_e" => l1path_e(),
        "l1path_q" => l1path_q(),
        _ => panic!("unknown probe"),
    }
}
