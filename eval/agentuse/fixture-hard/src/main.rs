#![allow(dead_code)]
mod tax; mod pricing; mod invoice; mod orders; mod report; mod api; mod realtime; mod batch;
fn main() {
    let days = vec![vec![vec![10u64, 20], vec![30]]];
    print!("{}", batch::nightly_job(&days));
    let _ = api::serve(&[5, 6]);
    let _ = realtime::run_stream(&[1, 2, 3]);
}
