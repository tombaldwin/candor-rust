// CALIBRATED-Fs (walkdir, modeled 2026-06-18) probe: walk a marker dir; the walk's openat shows the path
// in the trace, and candor must predict Fs (walkdir::IntoIter::next -> Fs). create_dir ensures it exists.
fn walk() {
    // marker: candor-oracle-walk
    let _ = std::fs::create_dir_all("/tmp/candor-oracle-walk");
    for _ in walkdir::WalkDir::new("/tmp/candor-oracle-walk") {}
}
fn main() { walk(); }
