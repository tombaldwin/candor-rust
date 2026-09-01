// CONTROL for pf_alias_chain: same chain, no let-alias hop at all.
fn put() {
    eprintln!("CFE put");
    let _ = std::fs::write("/tmp/pf-aliaschainc-92", b"x");
    eprintln!("CFX put");
}
fn body() { eprintln!("CFE body"); put(); eprintln!("CFX body"); }
fn main() { eprintln!("CFE main"); body(); eprintln!("CFX main"); }
