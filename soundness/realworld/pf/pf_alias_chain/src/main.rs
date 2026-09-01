// R99 mech-4 SHAPE: a TWO-HOP `let` fn-alias chain — `let w = std::fs::write; let v = w; v(..)`.
// The one-hop form already resolved; the chained form is the shape under test.
// PAIRED CONTROL: pf_alias_chain_ctl, identical but calling std::fs::write directly.
fn put() {
    eprintln!("CFE put");
    let w = std::fs::write;
    let v = w;
    let _ = v("/tmp/pf-aliaschain-927", b"x");
    eprintln!("CFX put");
}
fn body() { eprintln!("CFE body"); put(); eprintln!("CFX body"); }
fn main() { eprintln!("CFE main"); body(); eprintln!("CFX main"); }
