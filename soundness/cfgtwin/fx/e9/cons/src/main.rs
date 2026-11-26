fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c_e9_e" => cons_e9::c_e9_e(),
        _ => panic!("unknown probe"),
    }
}
