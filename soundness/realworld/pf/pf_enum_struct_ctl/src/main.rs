// CONTROL for pf_enum_struct: the same enum, the same payload type, in a TUPLE position.
trait Doer { fn go(&self); }
struct Writer;
impl Doer for Writer {
    fn go(&self) {
        eprintln!("CFE go");
        let _ = std::fs::write("/tmp/pf-enumstrc-9271", b"x");
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
