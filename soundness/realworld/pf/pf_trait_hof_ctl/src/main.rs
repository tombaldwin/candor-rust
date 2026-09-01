// CONTROL for pf_trait_hof: the dot-call spelling of the same dispatch. One variable — whether the
// trait method is named as a VALUE or called on a receiver.
trait Doer { fn go(&self); }
struct Writer;
impl Doer for Writer {
    fn go(&self) {
        eprintln!("CFE go");
        let _ = std::fs::write("/tmp/pf-traithofc-9271", b"x");
        eprintln!("CFX go");
    }
}

fn call_it(items: &[Writer]) {
    eprintln!("CFE call_it");
    items.iter().for_each(|d| d.go());
    eprintln!("CFX call_it");
}
fn main() { eprintln!("CFE main"); call_it(&[Writer]); eprintln!("CFX main"); }
