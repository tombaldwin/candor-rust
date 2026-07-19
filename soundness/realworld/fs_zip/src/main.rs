// HONESTY probe (UNCALIBRATED): the zip crate writes an archive to a marker path (open/openat). marker: candor-mk-zip
use std::io::Write;
fn archive() {
    let p = "/tmp/candor-mk-zip.zip";
    if let Ok(f) = std::fs::File::create(p) {
        let mut z = zip::ZipWriter::new(f);
        let _ = z.start_file::<_, ()>("m.txt", zip::write::SimpleFileOptions::default());
        let _ = z.write_all(b"marker");
        let _ = z.finish();
    }
}
fn main() { archive(); }
