// HONESTY probe: tempfile creates a marker-prefixed temp file; the kernel shows the openat. candor must
// predict Fs OR disclose (invisible/Unknown) — silent-pure here is a real under-report.
fn mk() {
    // marker: candor-oracle-temp
    let _ = tempfile::Builder::new().prefix("candor-oracle-temp").tempfile();
}
fn main() { mk(); }
