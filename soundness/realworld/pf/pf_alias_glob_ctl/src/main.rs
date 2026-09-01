// CONTROL for pf_alias_glob: identical in every respect except that the submodule names the item.
mod glb;
fn put() {
    eprintln!("CFE put");
    let _ = glb::write("/tmp/pf-aliasglobc-927", b"x");
    eprintln!("CFX put");
}
fn body() { eprintln!("CFE body"); put(); eprintln!("CFX body"); }
fn main() { eprintln!("CFE main"); body(); eprintln!("CFX main"); }
