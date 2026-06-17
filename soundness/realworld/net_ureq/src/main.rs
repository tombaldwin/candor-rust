// CALIBRATED-Net recall on a real HTTP crate: ureq connects to a marker IP. candor must predict Net.
fn fetch() {
    // marker: 192.0.2.3
    let _ = ureq::get("http://192.0.2.3:80/oracle").timeout(std::time::Duration::from_millis(300)).call();
}
fn main() { fetch(); }
