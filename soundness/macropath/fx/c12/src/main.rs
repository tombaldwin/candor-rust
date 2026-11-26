#![allow(unused)]

#[cfg(unix)] mod other;
#[cfg(not(unix))] mod other_p;
mod inner;
pub fn c12_e() { inner::go_c12(); }
pub fn c12_q() { inner::q_c12_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c12_e" => c12_e(),
        "c12_q" => c12_q(),
        _ => panic!("unknown probe"),
    }
}
