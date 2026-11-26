fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e9_e" => cf_e9::e9_e(),
        _ => panic!("unknown probe"),
    }
}
