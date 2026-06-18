// HONESTY probe (UNCALIBRATED Fs): the `fs_extra` crate reads a marker path. The kernel shows openat.
// candor doesn't model fs_extra, so it must DISCLOSE — silent-pure here is a real disclosure hole.
fn read() {
    // marker: /tmp/candor-oracle-fsextra
    let _ = fs_extra::file::read_to_string("/tmp/candor-oracle-fsextra");
}
fn main() { read(); }
