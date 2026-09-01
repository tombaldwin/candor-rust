// R99 mech-3 SHAPE: the effectful function is reachable ONLY through a `const` holding a fn item.
// main -> body -> put -> W(..), where `const W: fn(..) = std::fs::write`.
// PAIRED CONTROL: pf_alias_const_ctl, identical but calling std::fs::write directly.
const W: fn(String, Vec<u8>) -> std::io::Result<()> = std::fs::write;
fn put() {
    eprintln!("CFE put");
    let _ = W("/tmp/pf-aliasconst-927".to_string(), vec![120u8]);
    eprintln!("CFX put");
}
fn body() { eprintln!("CFE body"); put(); eprintln!("CFX body"); }
fn main() { eprintln!("CFE main"); body(); eprintln!("CFX main"); }
