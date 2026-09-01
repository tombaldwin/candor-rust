// CONTROL for pf_alias_const: same chain, std::fs::write called by name. One variable — the const.
fn put() {
    eprintln!("CFE put");
    let _ = std::fs::write("/tmp/pf-aliasconstc-92".to_string(), vec![120u8]);
    eprintln!("CFX put");
}
fn body() { eprintln!("CFE body"); put(); eprintln!("CFX body"); }
fn main() { eprintln!("CFE main"); body(); eprintln!("CFX main"); }
