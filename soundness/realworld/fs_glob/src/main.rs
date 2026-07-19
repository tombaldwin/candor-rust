// HONESTY probe (UNCALIBRATED): glob reads a marker directory (openat on the path). candor must predict Fs
// OR disclose Unknown/blind for the glob crate. marker: candor-mk-glob
fn scan() {
    let _ = std::fs::create_dir_all("/tmp/candor-mk-glob");
    if let Ok(paths) = glob::glob("/tmp/candor-mk-glob/*") { for _ in paths {} }
}
fn main() { scan(); }
