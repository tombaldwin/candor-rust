#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
mod a_bd2 { use fxdep_bd2::TryFromX; pub fn e() { let _ = String::try_from(1u8); } }
pub fn bd2_tryfrom_2018_e() { a_bd2::e() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bd2_tryfrom_2018_e" => bd2_tryfrom_2018_e(),
        _ => panic!("unknown probe"),
    }
}
