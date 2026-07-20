// main -> serve -> fetch -> ureq::get  (Net fires in the leaf; serve+fetch reach it only transitively)
fn fetch() {
    eprintln!("CFE fetch");
    let _ = ureq::get("http://192.0.2.1:80/pf").timeout(std::time::Duration::from_millis(300)).call();
    eprintln!("CFX fetch");
}
fn serve() { eprintln!("CFE serve"); fetch(); eprintln!("CFX serve"); }
fn main() { eprintln!("CFE main"); serve(); eprintln!("CFX main"); }
