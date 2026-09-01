// CONTROL for pf_oncelock_cb: the same opaque callable reached through a fn-typed PARAMETER, the
// sibling path R101 records as already correct. It should carry the effect or disclose Unknown.
fn fire(f: &dyn Fn()) {
    eprintln!("CFE fire");
    f();
    eprintln!("CFX fire");
}
fn main() {
    eprintln!("CFE main");
    fire(&|| { let _ = std::fs::write("/tmp/pf-oncecbc-9271", b"x"); });
    eprintln!("CFX main");
}
