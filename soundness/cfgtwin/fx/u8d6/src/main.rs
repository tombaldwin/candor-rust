fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "u8d6_e" => cf_u8d6::u8d6_e(),
        _ => panic!("unknown probe"),
    }
}
