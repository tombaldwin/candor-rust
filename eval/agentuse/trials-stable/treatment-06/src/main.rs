#![allow(dead_code)]

mod pricing;
mod invoice;
mod report;
mod monitoring;

fn main() {
    let orders = vec![vec!["WIDGET", "GADGET"], vec!["WIDGET"]];
    print!("{}", report::export_csv(&orders));
    let _ = monitoring::health_probe();
}
