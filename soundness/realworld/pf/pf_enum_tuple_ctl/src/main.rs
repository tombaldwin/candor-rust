// CONTROL for pf_enum_tuple: the same trait object, no enum in the path. One variable — the payload.
trait Doer { fn go(&self); }
struct Writer;
impl Doer for Writer {
    fn go(&self) {
        eprintln!("CFE go");
        let _ = std::fs::write("/tmp/pf-enumtupc-9271", b"x");
        eprintln!("CFX go");
    }
}

fn on_msg(f: Box<dyn Doer>) {
    eprintln!("CFE on_msg");
    f.go();
    eprintln!("CFX on_msg");
}
fn main() { eprintln!("CFE main"); on_msg(Box::new(Writer)); eprintln!("CFX main"); }
