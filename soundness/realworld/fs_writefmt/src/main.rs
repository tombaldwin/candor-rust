// CALIBRATED-Fs WRITE-FMT writer-side probe — the cross-engine write-fmt blind spot (silent in the rust
// deep+scan+swift engines; fixed: deep HOLE 2c, scan 0.5.18, swift 0.5.22). A custom `fmt::Write` whose
// `write_str` writes a marker FILE; `write!` drives it, so the kernel shows openat and candor-scan must
// predict Fs via the writer-side edge. A silent-pure here = the under-report we fixed — kernel-gated.
use std::fmt::Write as _;
struct FileWriter;
impl std::fmt::Write for FileWriter {
    fn write_str(&mut self, _s: &str) -> std::fmt::Result {
        // marker: /tmp/candor-oracle-writefmt
        let _ = std::fs::write("/tmp/candor-oracle-writefmt", b"x");
        Ok(())
    }
}
fn main() {
    let mut w = FileWriter;
    let _ = write!(w, "hi {}", 1);
}
