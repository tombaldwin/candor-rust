#![allow(unused)]
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
mod other;
mod inner;
pub fn c11a_e() { inner::go_c11a(); }
pub fn c11a_q() { inner::q_c11a_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c11a_e" => c11a_e(),
        "c11a_q" => c11a_q(),
        _ => panic!("unknown probe"),
    }
}
