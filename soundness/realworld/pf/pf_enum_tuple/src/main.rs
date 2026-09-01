// R77 SHAPE (tuple variant): the callable reaches the effect ONLY as a dispatch-typed payload of an
// enum TUPLE variant destructured in a match arm.
// PAIRED CONTROL: pf_enum_tuple_ctl — the same trait object bound directly as a parameter.
trait Doer { fn go(&self); }
struct Writer;
impl Doer for Writer {
    fn go(&self) {
        eprintln!("CFE go");
        let _ = std::fs::write("/tmp/pf-enumtup-9271", b"x");
        eprintln!("CFX go");
    }
}

enum Msg { Cb(Box<dyn Doer>) }
fn on_msg(m: Msg) {
    eprintln!("CFE on_msg");
    match m { Msg::Cb(f) => f.go() }
    eprintln!("CFX on_msg");
}
fn main() { eprintln!("CFE main"); on_msg(Msg::Cb(Box::new(Writer))); eprintln!("CFX main"); }
