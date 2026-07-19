// HONESTY probe (UNCALIBRATED): memmap2 opens+maps a marker file (open shows the path). marker: candor-mk-mmap
use std::fs::OpenOptions;
fn mapit() {
    let p = "/tmp/candor-mk-mmap";
    let _ = std::fs::write(p, b"x");
    if let Ok(f) = OpenOptions::new().read(true).open(p) {
        let _ = unsafe { memmap2::Mmap::map(&f) };
    }
}
fn main() { mapit(); }
