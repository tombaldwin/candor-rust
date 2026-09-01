// R99 mech-1 SHAPE: the host type is reachable ONLY through a submodule `pub use` of a std item.
// main -> body -> open_it -> facade::File::create   (nothing here names std::fs::File directly)
// PAIRED CONTROL: pf_alias_use_ctl, identical but spelling std::fs::File at the call site.
mod facade;
fn open_it() {
    eprintln!("CFE open_it");
    let _ = facade::File::create("/tmp/pf-aliasuse-9271");
    eprintln!("CFX open_it");
}
fn body() { eprintln!("CFE body"); open_it(); eprintln!("CFX body"); }
fn main() { eprintln!("CFE main"); body(); eprintln!("CFX main"); }
