// CONTROL for pf_alias_type: same chain, host named directly. One variable — the nominal alias.
fn open_it() {
    eprintln!("CFE open_it");
    let _ = std::fs::File::create("/tmp/pf-aliastypec-9271");
    eprintln!("CFX open_it");
}
fn body() { eprintln!("CFE body"); open_it(); eprintln!("CFX body"); }
fn main() { eprintln!("CFE main"); body(); eprintln!("CFX main"); }
