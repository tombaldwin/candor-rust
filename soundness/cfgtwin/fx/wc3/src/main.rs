#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
mod a_wc3 { use std::str::FromStr as _; use fxdep_wc3::Parse; pub fn e() { let _ = String::from_str("x"); } }
pub fn wc3_fromstr_as_e() { a_wc3::e() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "wc3_fromstr_as_e" => wc3_fromstr_as_e(),
        _ => panic!("unknown probe"),
    }
}
