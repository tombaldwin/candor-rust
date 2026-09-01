// R89 SHAPE: an ABSTRACT trait requirement passed as a first-class value to a HOF —
// `items.iter().for_each(Doer::go)`. `Doer::go` has no body, so pre-fix the edge matched no unit
// and evaporated with no Unknown anywhere.
// PAIRED CONTROL: pf_trait_hof_ctl — the same HOF, the same effect, a closure that dot-calls.
trait Doer { fn go(&self); }
struct Writer;
impl Doer for Writer {
    fn go(&self) {
        eprintln!("CFE go");
        let _ = std::fs::write("/tmp/pf-traithof-9271", b"x");
        eprintln!("CFX go");
    }
}

fn call_it(items: &[Writer]) {
    eprintln!("CFE call_it");
    items.iter().for_each(Doer::go);
    eprintln!("CFX call_it");
}
fn main() { eprintln!("CFE main"); call_it(&[Writer]); eprintln!("CFX main"); }
