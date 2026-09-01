// R77 SHAPE (struct variant): the callable reaches the effect ONLY as a FIELD of an enum STRUCT
// variant. R77 records this as a missing capability rather than a routing bug — no binder mechanism
// existed for struct-variant fields at all, for any payload type.
// PAIRED CONTROL: pf_enum_struct_ctl — the TUPLE-variant spelling of the same enum. One variable:
// struct-variant field vs tuple-variant position. (The no-enum arm is pf_enum_tuple_ctl.)
trait Doer { fn go(&self); }
struct Writer;
impl Doer for Writer {
    fn go(&self) {
        eprintln!("CFE go");
        let _ = std::fs::write("/tmp/pf-enumstr-9271", b"x");
        eprintln!("CFX go");
    }
}

enum Msg { CbField { f: Box<dyn Doer> } }
fn on_msg(m: Msg) {
    eprintln!("CFE on_msg");
    match m { Msg::CbField { f } => f.go() }
    eprintln!("CFX on_msg");
}
fn main() {
    eprintln!("CFE main");
    on_msg(Msg::CbField { f: Box::new(Writer) });
    eprintln!("CFX main");
}
