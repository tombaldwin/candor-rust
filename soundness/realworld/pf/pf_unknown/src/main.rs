// main -> app -> call -> minreq::get  (Net runs; minreq is uncalibrated, so candor should DISCLOSE Unknown
// on the transitive callers rather than read them silent-pure)
fn call() {
    eprintln!("CFE call");
    let _ = minreq::get("http://192.0.2.9:80/pf").with_timeout(1).send();
    eprintln!("CFX call");
}
fn app() { eprintln!("CFE app"); call(); eprintln!("CFX app"); }
fn main() { eprintln!("CFE main"); app(); eprintln!("CFX main"); }
