#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
mod a { use fxdep_l3e::FromIt; pub fn e() { let _ = String::from_iter(vec!['a']); } }
mod b { use std::iter::FromIterator; #[allow(unused_imports)] use fxdep_l3e::Other; pub fn q() { let _ = String::from_iter(vec!['a']); } }
pub fn l3e_fromiter_2018_e() { a::e() }
pub fn l3e_fromiter_import_2018_q() { b::q() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l3e_fromiter_2018_e" => l3e_fromiter_2018_e(),
        "l3e_fromiter_import_2018_q" => l3e_fromiter_import_2018_q(),
        _ => panic!("unknown probe"),
    }
}
