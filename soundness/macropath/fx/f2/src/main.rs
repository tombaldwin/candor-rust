#![allow(unused)]
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
mod other;
mod inner;
#[macro_export] macro_rules! mk { () => { () } }

pub fn f2_e() { inner::go_f2(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "f2_e" => f2_e(),
        _ => panic!("unknown probe"),
    }
}
