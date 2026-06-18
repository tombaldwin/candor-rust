// write!/writeln! to a custom Writer: `write!(w, ...)` desugars to `w.write_fmt(format_args!(...))`,
// whose default impl drives `w.write_str(...)` through the core::fmt machinery. An effectful local
// `impl fmt::Write` / `impl io::Write` reached only this way must be charged (no silent under-report);
// a std writer (String, Vec) stays pure.
#![allow(unused)]
use std::fmt::Write as _;
use std::io::Write as _;

fn sink() {
    let _ = std::fs::read_to_string("/etc/hostname"); // Fs
}

// effectful fmt::Write
struct LoudFmt;
impl std::fmt::Write for LoudFmt {
    fn write_str(&mut self, _s: &str) -> std::fmt::Result {
        sink();
        Ok(())
    }
}
fn via_write_fmt(w: &mut LoudFmt) {
    let _ = write!(w, "hi {}", 1); // Fs (write! -> write_fmt -> LoudFmt::write_str)
}

// effectful io::Write
struct LoudIo;
impl std::io::Write for LoudIo {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        sink();
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn via_write_io(w: &mut LoudIo) {
    let _ = write!(w, "hi {}", 1); // Fs (write! -> io::Write::write_fmt -> LoudIo::write)
}

// pure control: write! to a std String (non-local fmt::Write)
fn pure_write_string(s: &mut String) {
    let _ = write!(s, "hi {}", 1); // pure
}

fn main() {}
