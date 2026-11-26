#![allow(unused)]
#[macro_export] macro_rules! mkfn_l1expanded { ($n:ident) => { pub fn $n() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
#[macro_export] macro_rules! mkfnp_l1expanded { ($n:ident) => { pub fn $n() {} } }
mod inner;
pub fn l1expanded_e() { inner::p_inner_l1expanded(); }
pub fn l1expanded_q() { inner::q_inner_l1expanded(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l1expanded_e" => l1expanded_e(),
        "l1expanded_q" => l1expanded_q(),
        _ => panic!("unknown probe"),
    }
}
