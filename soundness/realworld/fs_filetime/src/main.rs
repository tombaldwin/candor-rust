// HONESTY probe (UNCALIBRATED): filetime touches a marker file (openat/utimensat). marker: candor-mk-ft
fn touch() {
    let p = "/tmp/candor-mk-ft";
    let _ = std::fs::write(p, b"x");
    let _ = filetime::set_file_mtime(p, filetime::FileTime::from_unix_time(1000, 0));
}
fn main() { touch(); }
