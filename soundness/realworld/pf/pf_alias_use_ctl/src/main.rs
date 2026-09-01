// CONTROL for pf_alias_use: same chain, same effect, the host named directly. One variable —
// the submodule re-export. If this driver ever fails, the harness is broken, not the engine.
fn open_it() {
    eprintln!("CFE open_it");
    let _ = std::fs::File::create("/tmp/pf-aliasusec-9271");
    eprintln!("CFX open_it");
}
fn body() { eprintln!("CFE body"); open_it(); eprintln!("CFX body"); }
fn main() { eprintln!("CFE main"); body(); eprintln!("CFX main"); }
