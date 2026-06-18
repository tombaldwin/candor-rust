// CALIBRATED-Fs (std) probe: std::fs writes+reads a marker path; the kernel shows openat, candor must predict Fs.
fn touch() {
    // marker: /tmp/candor-oracle-fs-std
    let _ = std::fs::write("/tmp/candor-oracle-fs-std", b"x");
    let _ = std::fs::read("/tmp/candor-oracle-fs-std");
}
fn main() { touch(); }
