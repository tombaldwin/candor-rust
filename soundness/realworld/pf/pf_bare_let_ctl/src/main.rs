// CONTROL for pf_bare_let: identical except the receiver is not bound first. One variable.
trait Doer { fn go(&self); }
struct Writer;
impl Doer for Writer {
    fn go(&self) {
        eprintln!("CFE go");
        let _ = std::fs::write("/tmp/pf-bareletc-9271", b"x");
        eprintln!("CFX go");
    }
}

struct App { single: Box<dyn Doer> }
impl App {
    fn run_direct(&self) {
        eprintln!("CFE run_direct");
        self.single.go();
        eprintln!("CFX run_direct");
    }
}
fn main() {
    eprintln!("CFE main");
    let a = App { single: Box::new(Writer) };
    a.run_direct();
    eprintln!("CFX main");
}
