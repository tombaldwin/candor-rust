mod api;
mod cache;
mod report;
mod service;

use service::Service;

fn main() {
    let svc = Service::new();
    println!("{}", api::get_one(&svc, "u1"));
    println!("{}", api::get_many(&svc, &["u1", "u2"]));
    print!("{}", report::build(&svc));
}
