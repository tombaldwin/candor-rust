// main -> load -> openf -> fs_err::read  (Fs fires in the leaf; load+openf reach it transitively)
fn openf() {
    eprintln!("CFE openf");
    let _ = fs_err::read("/tmp/pf-fs-marker-9271");
    eprintln!("CFX openf");
}
fn load() { eprintln!("CFE load"); openf(); eprintln!("CFX load"); }
fn main() { eprintln!("CFE main"); load(); eprintln!("CFX main"); }
