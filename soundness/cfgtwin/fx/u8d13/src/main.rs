fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "u8d13_e" => cf_u8d13::u8d13_e(),
        _ => panic!("unknown probe"),
    }
}
