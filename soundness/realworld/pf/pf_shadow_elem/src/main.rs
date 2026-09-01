// R100 SHAPE: a SELF-SHADOWING rebind whose RHS reads the shadowed name — `let xs = xs.clone();` —
// followed by a loop that calls a method on the element. Pre-fix, `elem_of[xs]` was removed before
// the RHS was walked, so the element type was lost and the caller vanished.
// PAIRED CONTROL: pf_shadow_elem_ctl, the NON-shadowing spelling (`let ys = xs.clone()`).
#[derive(Clone)]
struct Writer;
impl Writer {
    fn go(&self) {
        eprintln!("CFE go");
        let _ = std::fs::write("/tmp/pf-shadow-9271", b"x");
        eprintln!("CFX go");
    }
}
fn drive(xs: Vec<Writer>) {
    eprintln!("CFE drive");
    let xs = xs.clone();
    for x in &xs { x.go(); }
    eprintln!("CFX drive");
}
fn main() { eprintln!("CFE main"); drive(vec![Writer]); eprintln!("CFX main"); }
