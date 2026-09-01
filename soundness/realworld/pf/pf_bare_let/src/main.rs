// R88 SHAPE: the dispatch receiver is bound by a BARE, UNANNOTATED `let` before the call.
// PAIRED CONTROL: pf_bare_let_ctl — the same field, same trait object, called directly.
trait Doer { fn go(&self); }
struct Writer;
impl Doer for Writer {
    fn go(&self) {
        eprintln!("CFE go");
        let _ = std::fs::write("/tmp/pf-barelet-9271", b"x");
        eprintln!("CFX go");
    }
}

struct App { single: Box<dyn Doer> }
impl App {
    fn run_bound(&self) {
        eprintln!("CFE run_bound");
        let h = &self.single;
        h.go();
        eprintln!("CFX run_bound");
    }
}
fn main() {
    eprintln!("CFE main");
    let a = App { single: Box::new(Writer) };
    a.run_bound();
    eprintln!("CFX main");
}
