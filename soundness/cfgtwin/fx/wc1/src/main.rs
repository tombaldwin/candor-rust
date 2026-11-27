#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
mod a_wc1 { use fxdep_wc1::TryFromX; pub fn e() { let _ = String::try_from(1u8); } }
pub fn wc1_tryfrom_2021_e() { a_wc1::e() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "wc1_tryfrom_2021_e" => wc1_tryfrom_2021_e(),
        _ => panic!("unknown probe"),
    }
}
