#![allow(unused)]

mod defs;
mod other;
mod inner;
pub fn flimnestr_e() { inner::go_flimnestr(); }
pub fn flimnestr_q() { inner::q_flimnestr_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "flimnestr_e" => flimnestr_e(),
        "flimnestr_q" => flimnestr_q(),
        _ => panic!("unknown probe"),
    }
}
