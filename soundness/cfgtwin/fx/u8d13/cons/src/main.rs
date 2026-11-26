fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c_u8d13_e" => cons_u8d13::c_u8d13_e(),
        _ => panic!("unknown probe"),
    }
}
