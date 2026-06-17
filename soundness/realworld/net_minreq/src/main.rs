// HONESTY probe: a real HTTP crate (minreq) connecting to a marker IP. The kernel will show the
// connect() — so the program demonstrably does Net. candor must therefore predict Net OR disclose
// Unknown/blind (honest). Silent-pure here = a real silent under-report (the worst bug).
fn fetch() {
    // marker: 192.0.2.2
    let _ = minreq::get("http://192.0.2.2:80/oracle").with_timeout(1).send();
}

fn main() {
    fetch();
}
