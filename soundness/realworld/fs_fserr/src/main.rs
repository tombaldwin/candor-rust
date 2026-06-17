// CALIBRATED-Fs recall probe: fs-err (a thin std::fs wrapper) opens a marker path. The kernel shows the
// openat; candor must predict Fs.
fn read_file() {
    // marker: candor-oracle-fs-marker
    let _ = fs_err::read("/tmp/candor-oracle-fs-marker");
}

fn main() {
    read_file();
}
