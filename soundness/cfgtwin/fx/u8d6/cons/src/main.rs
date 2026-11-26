fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c_u8d6_e" => cons_u8d6::c_u8d6_e(),
        _ => panic!("unknown probe"),
    }
}
