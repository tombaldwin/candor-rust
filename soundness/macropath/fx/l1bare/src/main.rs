#![allow(unused)]
mod inner; mod defs;
pub use inner::{l1bare_d, l1bare_dq};
pub fn l1bare_e() { inner::p_inner_l1bare(); }
pub fn l1bare_q() { inner::q_inner_l1bare(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1bare_e" => l1bare_e(),
        "l1bare_q" => l1bare_q(),
        "l1bare_d" => l1bare_d(),
        "l1bare_dq" => l1bare_dq(),
        _ => panic!("unknown probe"),
    }
}
