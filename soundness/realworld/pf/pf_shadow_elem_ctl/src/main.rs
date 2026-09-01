// CONTROL for pf_shadow_elem: identical but the rebind uses a FRESH name. One variable — shadowing.
#[derive(Clone)]
struct Writer;
impl Writer {
    fn go(&self) {
        eprintln!("CFE go");
        let _ = std::fs::write("/tmp/pf-shadowc-9271", b"x");
        eprintln!("CFX go");
    }
}
fn drive(xs: Vec<Writer>) {
    eprintln!("CFE drive");
    let ys = xs.clone();
    for y in &ys { y.go(); }
    eprintln!("CFX drive");
}
fn main() { eprintln!("CFE main"); drive(vec![Writer]); eprintln!("CFX main"); }
