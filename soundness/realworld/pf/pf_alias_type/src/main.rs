// R99 mech-2 SHAPE: the host type is reachable ONLY through a NOMINAL `pub type` alias.
// main -> body -> open_it -> Sink::create, where `Sink` aliases std::fs::File.
// PAIRED CONTROL: pf_alias_type_ctl, identical but without the alias.
pub type Sink = std::fs::File;
fn open_it() {
    eprintln!("CFE open_it");
    let _ = Sink::create("/tmp/pf-aliastype-9271");
    eprintln!("CFX open_it");
}
fn body() { eprintln!("CFE body"); open_it(); eprintln!("CFX body"); }
fn main() { eprintln!("CFE main"); body(); eprintln!("CFX main"); }
